use std::fs;
use std::io::{ErrorKind, Read, Seek};
use std::path::{Path, PathBuf};

use chrono::Local;
use image::ImageFormat;

use crate::config::{Config, expand_tilde};
use crate::dbus_client::FrontendClient;
use crate::error::{PinshotError, Result};

const DEFAULT_FILENAME_PREFIX: &str = "pinshot";
const EXPORT_TIME_FORMAT: &str = "%Y%m%d-%H%M%S";

/// 框选完成后用户明确选择的截图交付动作
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScreenshotAction {
    Copy,
    Pin,
    Edit,
    Save,
}

impl ScreenshotAction {
    fn as_str(self) -> &'static str {
        match self {
            Self::Copy => "copy",
            Self::Pin => "pin",
            Self::Edit => "edit",
            Self::Save => "save",
        }
    }

    pub(crate) fn from_wire(action: &str) -> Result<Self> {
        match action {
            "copy" => Ok(Self::Copy),
            "pin" => Ok(Self::Pin),
            "edit" => Ok(Self::Edit),
            "save" => Ok(Self::Save),
            _ => Err(PinshotError::Dbus(format!(
                "前端返回无效的截图动作：{action}"
            ))),
        }
    }
}

/// 虚拟桌面逻辑坐标 (x, y, width, height)，可用于把选区交给长截图或 GIF 录制
pub type StageRegion = (i32, i32, i32, i32);

/// 一次长截图只沿一个轴拼接，允许向该轴的两端扩展
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum ScrollAxis {
    #[default]
    Vertical,
    Horizontal,
}

/// 区域截图确认后交付 PNG，或把原选区交给连续录制
pub enum ScreenshotSelection {
    Image(CapturedImage),
    Longshot(StageRegion, ScrollAxis),
    Gif(StageRegion),
}

/// 区域截屏成功后的输出图像和交付动作
#[derive(Debug)]
pub struct CapturedImage {
    pub action: ScreenshotAction,
    pub png_bytes: Vec<u8>,
    pub source_scale: f64,
    /// 原始选区左上角逻辑坐标，贴图时用于原位显示
    pub initial_position: Option<(i32, i32)>,
}

/// 扩展首选项中的 Pinshot 截图输出策略
#[derive(Debug)]
pub struct ScreenshotSettings {
    pub auto_save: bool,
    pub save_dir: String,
    pub filename_prefix: String,
    pub notify: bool,
}

impl ScreenshotSettings {
    /// 以本地保存时间为后缀导出 PNG；同名时追加序号，不覆盖旧图
    pub fn export_png(&self, cfg: &Config, png: impl Read) -> Result<PathBuf> {
        let timestamp = Local::now().format(EXPORT_TIME_FORMAT).to_string();
        self.export_png_with_timestamp(cfg, png, &timestamp)
    }

    fn export_png_with_timestamp(
        &self,
        cfg: &Config,
        mut png: impl Read,
        timestamp: &str,
    ) -> Result<PathBuf> {
        let stem = screenshot_stem(&self.filename_prefix, timestamp)?;
        let dir = self.export_dir(cfg)?;
        fs::create_dir_all(&dir).map_err(|err| PinshotError::io(&dir, err))?;
        let mut temp =
            tempfile::NamedTempFile::new_in(&dir).map_err(|err| PinshotError::io(&dir, err))?;
        std::io::copy(&mut png, temp.as_file_mut())
            .map_err(|err| PinshotError::io(temp.path(), err))?;
        persist_unique_image(temp, &dir, &stem, "png")
    }

    /// 解析截图与录制共用的保存目录
    pub fn export_dir(&self, cfg: &Config) -> Result<PathBuf> {
        if self.save_dir.trim().is_empty() {
            cfg.resolved_save_dir()
        } else {
            let path = expand_tilde(self.save_dir.trim())?;
            if !path.is_absolute() {
                return Err(PinshotError::Config(
                    "截图保存目录必须为绝对路径或以 ~/ 开头".to_string(),
                ));
            }
            Ok(path)
        }
    }

    /// 生成 GIF 文件名主体；与截图共用前缀，保存时添加当前本地时间
    #[cfg(feature = "longshot")]
    pub fn gif_stem(&self) -> Result<String> {
        let prefix = strip_extension(&self.filename_prefix, "gif");
        let timestamp = Local::now().format(EXPORT_TIME_FORMAT).to_string();
        if prefix.is_empty() && !self.filename_prefix.trim().is_empty() {
            return Err(PinshotError::Config("GIF 文件名前缀不能为空".into()));
        }
        screenshot_stem(prefix, &timestamp)
    }
}

fn screenshot_stem(prefix: &str, timestamp: &str) -> Result<String> {
    let prefix = prefix.trim();
    let prefix = if prefix.is_empty() {
        DEFAULT_FILENAME_PREFIX
    } else {
        prefix
    };
    let prefix = strip_extension(prefix, "png");
    if prefix.is_empty() || prefix == "." || prefix == ".." || prefix.contains(['/', '\0']) {
        return Err(PinshotError::Config(
            "截图文件名前缀不能为空或包含路径".to_string(),
        ));
    }
    Ok(format!("{prefix}-{timestamp}"))
}

fn strip_extension<'a>(prefix: &'a str, extension: &str) -> &'a str {
    let prefix = prefix.trim();
    match prefix.rsplit_once('.') {
        Some((name, ext)) if ext.eq_ignore_ascii_case(extension) => name.trim_end(),
        _ => prefix,
    }
}

pub(crate) fn persist_unique_image(
    mut temp: tempfile::NamedTempFile,
    dir: &Path,
    stem: &str,
    extension: &str,
) -> Result<PathBuf> {
    let mut suffix = 0;
    loop {
        let target = if suffix == 0 {
            dir.join(format!("{stem}.{extension}"))
        } else {
            dir.join(format!("{stem}-{suffix}.{extension}"))
        };
        match temp.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(err) if err.error.kind() == ErrorKind::AlreadyExists => {
                temp = err.file;
                suffix += 1;
            }
            Err(err) => return Err(PinshotError::io(&target, err.error)),
        }
    }
}

/// 直接截取指针所在显示器的全屏画面；匿名临时文件只用来传输 PNG
pub fn capture_fullscreen(frontend: &FrontendClient) -> Result<Option<CapturedImage>> {
    let temp_dir = std::env::temp_dir();
    let mut file = tempfile::tempfile().map_err(|err| PinshotError::io(&temp_dir, err))?;
    let source_scale = frontend.capture_fullscreen(&file)?;
    if source_scale == 0.0 {
        return Ok(None);
    }
    if !source_scale.is_finite() || source_scale < 0.0 {
        return Err(PinshotError::Dbus(format!(
            "无效的截图缩放：{source_scale}"
        )));
    }
    let mut png_bytes = Vec::new();
    file.rewind()
        .map_err(|err| PinshotError::io(&temp_dir, err))?;
    file.read_to_end(&mut png_bytes)
        .map_err(|err| PinshotError::io(&temp_dir, err))?;
    Ok(Some(CapturedImage {
        action: ScreenshotAction::Copy,
        png_bytes,
        source_scale,
        initial_position: None,
    }))
}

/// 打开 Pinshot 框选；匿名临时文件只用来传输 PNG，不在截图目录落盘
pub fn capture_interactive_region(
    frontend: &FrontendClient,
    default_action: ScreenshotAction,
) -> Result<Option<ScreenshotSelection>> {
    let temp_dir = std::env::temp_dir();
    let mut file = tempfile::tempfile().map_err(|err| PinshotError::io(&temp_dir, err))?;
    let (source_scale, action, region) =
        frontend.select_screenshot(&file, default_action.as_str())?;
    let valid_region =
        cfg!(feature = "longshot") && source_scale == 0.0 && region.2 > 0 && region.3 > 0;
    match action.as_str() {
        "longshot" if valid_region => {
            return Ok(Some(ScreenshotSelection::Longshot(
                region,
                ScrollAxis::Vertical,
            )));
        }
        "longshot-horizontal" if valid_region => {
            return Ok(Some(ScreenshotSelection::Longshot(
                region,
                ScrollAxis::Horizontal,
            )));
        }
        "longshot" | "longshot-horizontal" => {
            return Err(PinshotError::Dbus("前端返回无效的长截图选区".into()));
        }
        "gif" if valid_region => return Ok(Some(ScreenshotSelection::Gif(region))),
        "gif" => return Err(PinshotError::Dbus("前端返回无效的 GIF 选区".into())),
        _ => {}
    }
    if source_scale == 0.0 {
        return Ok(None);
    }
    if !source_scale.is_finite() || source_scale < 0.0 {
        return Err(PinshotError::Dbus(format!(
            "无效的截图缩放：{source_scale}"
        )));
    }
    let action = ScreenshotAction::from_wire(&action)?;
    let mut png_bytes = Vec::new();
    file.rewind()
        .map_err(|err| PinshotError::io(&temp_dir, err))?;
    file.read_to_end(&mut png_bytes)
        .map_err(|err| PinshotError::io(&temp_dir, err))?;
    let initial_position = (region.2 > 0 && region.3 > 0).then_some((region.0, region.1));
    Ok(Some(ScreenshotSelection::Image(CapturedImage {
        action,
        png_bytes,
        source_scale,
        initial_position,
    })))
}

/// 确保图像字节流为标准 PNG（已是 PNG 时零重编码开销直接复用）
pub(crate) fn ensure_png_bytes(raw: &[u8]) -> Result<Vec<u8>> {
    const PNG_MAGIC: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if raw.len() >= PNG_MAGIC.len() && &raw[..PNG_MAGIC.len()] == PNG_MAGIC {
        // 校验 PNG 头部有效性后再返回，防止损坏数据进入缓存
        image::load_from_memory_with_format(raw, ImageFormat::Png)?;
        return Ok(raw.to_vec());
    }

    let img = image::load_from_memory(raw)?;
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, ImageFormat::Png)?;
    Ok(out.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAVED_AT: &str = "20261121-214530";

    #[test]
    fn screenshot_action_round_trips_without_an_implicit_default() {
        for action in [
            ScreenshotAction::Copy,
            ScreenshotAction::Pin,
            ScreenshotAction::Edit,
            ScreenshotAction::Save,
        ] {
            assert_eq!(
                ScreenshotAction::from_wire(action.as_str()).unwrap(),
                action
            );
        }
        assert!(ScreenshotAction::from_wire("").is_err());
        assert!(ScreenshotAction::from_wire("finish").is_err());
    }

    #[test]
    fn filename_uses_timestamp_with_default_or_custom_prefix() {
        assert_eq!(
            screenshot_stem("", SAVED_AT).unwrap(),
            "pinshot-20261121-214530"
        );
        assert_eq!(
            screenshot_stem("笔记.PNG", SAVED_AT).unwrap(),
            "笔记-20261121-214530"
        );
        assert!(screenshot_stem("../outside", SAVED_AT).is_err());
        assert!(screenshot_stem("..", SAVED_AT).is_err());
        assert!(screenshot_stem(".png", SAVED_AT).is_err());
    }

    #[test]
    fn timestamp_uses_the_supplied_local_clock_fields() {
        let saved_at = chrono::DateTime::parse_from_rfc3339("2026-11-21T21:45:30+08:00").unwrap();
        assert_eq!(saved_at.format(EXPORT_TIME_FORMAT).to_string(), SAVED_AT);
    }

    #[test]
    fn custom_export_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("我的截图");
        let settings = ScreenshotSettings {
            auto_save: true,
            save_dir: destination.to_string_lossy().into_owned(),
            filename_prefix: "截图".into(),
            notify: false,
        };
        let cfg = Config::default();
        let first = settings
            .export_png_with_timestamp(&cfg, &b"first"[..], SAVED_AT)
            .unwrap();
        let second = settings
            .export_png_with_timestamp(&cfg, &b"second"[..], SAVED_AT)
            .unwrap();
        assert_eq!(first, destination.join("截图-20261121-214530.png"));
        assert_eq!(second, destination.join("截图-20261121-214530-1.png"));
        assert_eq!(fs::read(first).unwrap(), b"first");
        assert_eq!(fs::read(second).unwrap(), b"second");
        assert_eq!(fs::read_dir(destination).unwrap().count(), 2);
    }

    #[test]
    fn empty_directory_uses_existing_pinshot_configuration() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = Config::default();
        cfg.general.save_dir = dir.path().to_string_lossy().into_owned();
        let settings = ScreenshotSettings {
            auto_save: false,
            save_dir: String::new(),
            filename_prefix: "pinshot".into(),
            notify: false,
        };
        let saved = settings
            .export_png_with_timestamp(&cfg, &b"png"[..], SAVED_AT)
            .unwrap();
        assert_eq!(saved, dir.path().join("pinshot-20261121-214530.png"));
    }
}

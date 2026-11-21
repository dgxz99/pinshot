use std::fs;
use std::io::{ErrorKind, Read, Seek};
use std::path::{Path, PathBuf};

use image::ImageFormat;

use crate::config::{Config, expand_tilde};
use crate::dbus_client::FrontendClient;
use crate::error::{PinshotError, Result};
use crate::store::unix_now_secs;

/// 区域截屏成功后的输出图像
#[derive(Debug)]
pub struct CapturedImage {
    pub png_bytes: Vec<u8>,
    pub source_scale: f64,
}

/// 扩展首选项中的 Pinshot 截图输出策略
#[derive(Debug)]
pub struct ScreenshotSettings {
    pub auto_save: bool,
    pub save_dir: String,
    pub filename_template: String,
    pub notify: bool,
}

impl ScreenshotSettings {
    /// 导出一份永久文件；同名时保留旧图并为新图追加数字后缀
    pub fn export_png(
        &self,
        cfg: &Config,
        id: Option<&str>,
        mut png: impl Read,
    ) -> Result<PathBuf> {
        let stem = screenshot_stem(&self.filename_template, id)?;
        let dir = if self.save_dir.trim().is_empty() {
            cfg.resolved_save_dir()?
        } else {
            let path = expand_tilde(self.save_dir.trim())?;
            if !path.is_absolute() {
                return Err(PinshotError::Config(
                    "截图保存目录必须为绝对路径或以 ~/ 开头".to_string(),
                ));
            }
            path
        };
        fs::create_dir_all(&dir).map_err(|err| PinshotError::io(&dir, err))?;
        let mut temp =
            tempfile::NamedTempFile::new_in(&dir).map_err(|err| PinshotError::io(&dir, err))?;
        std::io::copy(&mut png, temp.as_file_mut())
            .map_err(|err| PinshotError::io(temp.path(), err))?;
        persist_unique_png(temp, &dir, &stem)
    }
}

fn screenshot_stem(template: &str, id: Option<&str>) -> Result<String> {
    let template = template.trim();
    let template = if template.is_empty() {
        "pinshot-{id}"
    } else {
        template
    };
    let template = template.strip_suffix(".png").unwrap_or(template);
    let time_id = unix_now_secs().to_string();
    let stem = template.replace("{id}", id.unwrap_or(&time_id));
    if stem.trim().is_empty() || stem == "." || stem == ".." || stem.contains(['/', '\0']) {
        return Err(PinshotError::Config(
            "截图文件名必须是单个文件名，不能包含路径".to_string(),
        ));
    }
    Ok(stem)
}

fn persist_unique_png(
    mut temp: tempfile::NamedTempFile,
    dir: &Path,
    stem: &str,
) -> Result<PathBuf> {
    let mut suffix = 0;
    loop {
        let target = if suffix == 0 {
            dir.join(format!("{stem}.png"))
        } else {
            dir.join(format!("{stem}-{suffix}.png"))
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

/// 打开 GNOME 原生截图界面；匿名临时文件只用来传输 PNG，不在截图目录落盘
pub fn capture_interactive_region(frontend: &FrontendClient) -> Result<Option<CapturedImage>> {
    let temp_dir = std::env::temp_dir();
    let mut file = tempfile::tempfile().map_err(|err| PinshotError::io(&temp_dir, err))?;
    let source_scale = frontend.capture_screenshot(&file)?;
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
        png_bytes,
        source_scale,
    }))
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

    #[test]
    fn filename_accepts_custom_names_and_rejects_paths() {
        assert_eq!(screenshot_stem("", Some("42")).unwrap(), "pinshot-42");
        assert_eq!(
            screenshot_stem("笔记-{id}.png", Some("42")).unwrap(),
            "笔记-42"
        );
        assert!(screenshot_stem("../outside", Some("42")).is_err());
        assert!(screenshot_stem("..", None).is_err());
        assert!(screenshot_stem(".png", None).is_err());
    }

    #[test]
    fn custom_export_keeps_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let destination = dir.path().join("我的截图");
        let settings = ScreenshotSettings {
            auto_save: true,
            save_dir: destination.to_string_lossy().into_owned(),
            filename_template: "截图".into(),
            notify: false,
        };
        let cfg = Config::default();
        let first = settings.export_png(&cfg, None, &b"first"[..]).unwrap();
        let second = settings.export_png(&cfg, None, &b"second"[..]).unwrap();
        assert_eq!(first, destination.join("截图.png"));
        assert_eq!(second, destination.join("截图-1.png"));
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
            filename_template: "pinshot-{id}".into(),
            notify: false,
        };
        let saved = settings.export_png(&cfg, Some("42"), &b"png"[..]).unwrap();
        assert_eq!(saved, dir.path().join("pinshot-42.png"));
    }
}

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{PinshotError, Result};

const DEFAULT_SAVE_DIR: &str = "~/Pictures/Pinshots";

/// 默认生成的配置文件模板（包含中文注释，便于用户手工调整）
const DEFAULT_CONFIG_TEMPLATE: &str = r#"[general]
save_dir = "~/Pictures/Pinshots"
restore_on_startup = false
notify_on_save = true

[cache]
max_size_mb = 200
max_age_days = 14

[pin]
default_opacity = 1.0
min_scale = 0.2
max_scale = 5.0
zoom_step = 0.1
opacity_step = 0.08
mini_size_px = 64
min_visible_edge_px = 32
max_texture_edge_px = 4096
hide_on_fullscreen = true

[text_card]
theme = "auto"
max_width_px = 680
max_lines = 80
prose_font = "Sans 12"
code_font = "Monospace 11"
padding_px = 16
"#;

/// 应用总配置（对应 `~/.config/pinshot/config.toml`）
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    pub general: GeneralConfig,
    pub cache: CacheConfig,
    pub pin: PinConfig,
    pub text_card: TextCardConfig,
}

/// 通用行为配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GeneralConfig {
    /// 用户主动保存截图时的目标目录
    pub save_dir: String,
    /// 登录重启后是否自动恢复上次活跃贴图
    pub restore_on_startup: bool,
    /// 保存截图后是否发送桌面通知
    pub notify_on_save: bool,
}

impl Default for GeneralConfig {
    fn default() -> Self {
        Self {
            save_dir: DEFAULT_SAVE_DIR.to_string(),
            restore_on_startup: false,
            notify_on_save: true,
        }
    }
}

/// 缓存与双阈值滚动清理配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CacheConfig {
    /// 缓存目录最大磁盘占用（MB）
    pub max_size_mb: u64,
    /// 缓存最长保留天数
    pub max_age_days: u64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            max_size_mb: 200,
            max_age_days: 14,
        }
    }
}

/// 贴图前端展示与交互参数
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PinConfig {
    pub default_opacity: f64,
    pub min_scale: f64,
    pub max_scale: f64,
    pub zoom_step: f64,
    pub opacity_step: f64,
    /// 迷你缩略图逻辑边长（像素）
    pub mini_size_px: u32,
    /// 拖拽防丢至少留在屏幕内的边缘像素
    pub min_visible_edge_px: u32,
    /// 前端纹理渲染单边最大像素上限
    pub max_texture_edge_px: u32,
    /// 全屏应用出现时是否自动隐藏该显示器贴图
    pub hide_on_fullscreen: bool,
}

impl Default for PinConfig {
    fn default() -> Self {
        Self {
            default_opacity: 1.0,
            min_scale: 0.2,
            max_scale: 5.0,
            zoom_step: 0.1,
            opacity_step: 0.08,
            mini_size_px: 64,
            min_visible_edge_px: 32,
            max_texture_edge_px: 4096,
            hide_on_fullscreen: true,
        }
    }
}

/// 剪贴板文字卡片主题模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TextCardTheme {
    #[default]
    Auto,
    Dark,
    Light,
}

/// 剪贴板纯文本渲染配置
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TextCardConfig {
    pub theme: TextCardTheme,
    pub max_width_px: u32,
    /// 超长文本截断保护行数
    pub max_lines: usize,
    pub prose_font: String,
    pub code_font: String,
    pub padding_px: u32,
}

impl Default for TextCardConfig {
    fn default() -> Self {
        Self {
            theme: TextCardTheme::Auto,
            max_width_px: 680,
            max_lines: 80,
            prose_font: "Sans 12".to_string(),
            code_font: "Monospace 11".to_string(),
            padding_px: 16,
        }
    }
}

impl Config {
    /// 返回默认配置文件路径：`~/.config/pinshot/config.toml`
    pub fn default_config_path() -> Result<PathBuf> {
        let base = dirs::config_dir()
            .ok_or_else(|| PinshotError::HomeDir("无法获取 XDG config 目录".to_string()))?;
        Ok(base.join("pinshot").join("config.toml"))
    }

    /// 从默认路径加载配置；若文件不存在则自动写入默认模板并返回默认配置
    pub fn load_or_init() -> Result<Self> {
        let path = Self::default_config_path()?;
        Self::load_or_init_at(&path)
    }

    /// 从指定路径加载配置；若文件不存在则自动创建父目录并写入默认模板
    pub fn load_or_init_at(path: &Path) -> Result<Self> {
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).map_err(|e| PinshotError::io(parent, e))?;
            }
            fs::write(path, DEFAULT_CONFIG_TEMPLATE).map_err(|e| PinshotError::io(path, e))?;
        }
        Self::load_from_path(path)
    }

    /// 仅从指定路径读取并解析配置，不修改磁盘文件
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let content = fs::read_to_string(path).map_err(|e| PinshotError::io(path, e))?;
        let mut cfg: Config =
            toml::from_str(&content).map_err(|e| PinshotError::Config(e.to_string()))?;
        cfg.normalize();
        Ok(cfg)
    }

    /// 展开 `general.save_dir` 中的 `~` 前缀，返回实际保存目录路径
    pub fn resolved_save_dir(&self) -> Result<PathBuf> {
        let configured = self.general.save_dir.trim();
        let directory = match configured {
            // 旧版自动生成的默认目录跟随新默认值，不改写用户文件
            "" | "~/Pictures/Screenshots" | "~/Pictures/Screenshots/" => DEFAULT_SAVE_DIR,
            _ => configured,
        };
        expand_tilde(directory)
    }

    /// 规范化配置边界值，防止非法参数导致运行时除零或异常
    fn normalize(&mut self) {
        self.pin.default_opacity = self.pin.default_opacity.clamp(0.15, 1.0);
        if self.pin.min_scale <= 0.05 {
            self.pin.min_scale = 0.05;
        }
        if self.pin.max_scale < self.pin.min_scale {
            self.pin.max_scale = self.pin.min_scale;
        }
        if self.pin.zoom_step <= 0.01 {
            self.pin.zoom_step = 0.05;
        }
        if self.pin.opacity_step <= 0.01 {
            self.pin.opacity_step = 0.05;
        }
        if self.pin.mini_size_px < 24 {
            self.pin.mini_size_px = 24;
        }
        if self.pin.max_texture_edge_px < 512 {
            self.pin.max_texture_edge_px = 512;
        }
        if self.text_card.max_width_px < 160 {
            self.text_card.max_width_px = 160;
        }
        if self.text_card.max_lines == 0 {
            self.text_card.max_lines = 1;
        }
    }
}

/// 展开路径开头的 `~` 或 `~/`
pub fn expand_tilde(raw: &str) -> Result<PathBuf> {
    let Some(rest) = raw.strip_prefix('~') else {
        return Ok(PathBuf::from(raw));
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return Ok(PathBuf::from(raw));
    }
    let home = dirs::home_dir()
        .ok_or_else(|| PinshotError::HomeDir("无法获取用户 HOME 目录".to_string()))?;
    Ok(match rest.strip_prefix('/') {
        Some(sub) => home.join(sub),
        None => home,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn default_template_parses_to_default_config() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = dir.path().join("pinshot").join("config.toml");

        let cfg = Config::load_or_init_at(&path).expect("初始化并读取配置失败");
        assert!(path.exists(), "应自动写入默认配置文件");
        assert_eq!(cfg, Config::default());
    }

    #[test]
    fn partial_override_keeps_other_defaults() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[cache]
max_size_mb = 512

[text_card]
theme = "dark"
"#,
        )
        .expect("写入配置失败");

        let cfg = Config::load_from_path(&path).expect("解析部分配置失败");
        assert_eq!(cfg.cache.max_size_mb, 512);
        assert_eq!(cfg.cache.max_age_days, 14);
        assert_eq!(cfg.text_card.theme, TextCardTheme::Dark);
        assert_eq!(cfg.pin.mini_size_px, 64);
    }

    #[test]
    fn normalize_clamps_invalid_bounds() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = dir.path().join("config.toml");
        fs::write(
            &path,
            r#"
[pin]
default_opacity = 3.5
min_scale = 2.0
max_scale = 0.5
"#,
        )
        .expect("写入配置失败");

        let cfg = Config::load_from_path(&path).expect("解析配置失败");
        assert_eq!(cfg.pin.default_opacity, 1.0);
        assert_eq!(cfg.pin.min_scale, 2.0);
        assert_eq!(cfg.pin.max_scale, 2.0);
    }

    #[test]
    fn invalid_toml_returns_config_error() {
        let dir = TempDir::new().expect("创建临时目录失败");
        let path = dir.path().join("config.toml");
        fs::write(&path, "[general\ninvalid =").expect("写入配置失败");

        let err = Config::load_from_path(&path).expect_err("非法 TOML 应报错");
        assert!(matches!(err, PinshotError::Config(_)));
    }

    #[test]
    fn old_default_save_dir_uses_pinshots_without_changing_custom_paths() {
        let mut cfg = Config::default();
        let default = cfg.resolved_save_dir().unwrap();
        assert!(default.ends_with("Pictures/Pinshots"));
        for value in ["~/Pictures/Screenshots", "~/Pictures/Screenshots/", ""] {
            cfg.general.save_dir = value.to_string();
            assert_eq!(cfg.resolved_save_dir().unwrap(), default);
        }
        cfg.general.save_dir = "/tmp/Custom Screenshots".to_string();
        assert_eq!(
            cfg.resolved_save_dir().unwrap(),
            PathBuf::from("/tmp/Custom Screenshots")
        );
    }

    #[test]
    fn expand_tilde_resolves_home_prefix() {
        let expanded = expand_tilde(DEFAULT_SAVE_DIR).expect("展开波浪号失败");
        assert!(expanded.ends_with("Pictures/Pinshots"));
        assert!(!expanded.to_string_lossy().starts_with('~'));

        let abs = expand_tilde("/tmp/pinshot-save").expect("绝对路径解析失败");
        assert_eq!(abs, PathBuf::from("/tmp/pinshot-save"));
    }
}

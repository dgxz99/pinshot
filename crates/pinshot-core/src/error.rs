use std::path::PathBuf;

/// Pinshot 核心统一错误类型
#[derive(Debug, thiserror::Error)]
pub enum PinshotError {
    /// 文件系统 I/O 错误
    #[error("I/O 操作失败 ({path}): {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    /// 配置文件解析或序列化失败
    #[error("配置解析失败: {0}")]
    Config(String),

    /// JSON 序列化或反序列化失败
    #[error("JSON 处理失败: {0}")]
    Json(#[from] serde_json::Error),

    /// 图像编解码或处理失败
    #[error("图像处理失败: {0}")]
    Image(#[from] image::ImageError),

    /// D-Bus 通信失败
    #[error("前端 D-Bus 通信失败: {0}")]
    Dbus(String),

    /// 未找到指定历史或贴图记录
    #[error("未找到指定记录: {0}")]
    NotFound(String),

    /// 外部命令执行失败
    #[error("外部命令 `{cmd}` 执行失败: {detail}")]
    ExternalCommand { cmd: String, detail: String },

    /// 无法解析 XDG / HOME 目录
    #[error("无法定位用户目录: {0}")]
    HomeDir(String),
}

/// Pinshot 核心统一结果类型
pub type Result<T> = std::result::Result<T, PinshotError>;

impl PinshotError {
    /// 构造带路径上下文的 I/O 错误
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }
}

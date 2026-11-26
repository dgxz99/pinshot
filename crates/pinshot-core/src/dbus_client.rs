use std::fs::File;
use std::io::{Seek, Write};
use std::os::fd::AsFd;

use zbus::blocking::Connection;
use zbus::zvariant::Fd;

use crate::capture::{ScreenshotSettings, StageRegion};
use crate::error::{PinshotError, Result};
use crate::protocol::{ActivePinInfo, FRONTEND_BUS_NAME, PinPayload};

#[zbus::proxy(
    interface = "io.github.dgxz99.Pinshot.Frontend1",
    default_service = "io.github.dgxz99.Pinshot.Frontend",
    default_path = "/io/github/dgxz99/Pinshot/Frontend",
    gen_async = false,
    gen_blocking = true
)]
trait Frontend1 {
    fn ping(&self) -> zbus::Result<String>;
    fn create_pin(&self, payload_json: &str) -> zbus::Result<bool>;
    fn update_pin(&self, id: &str, payload_json: &str) -> zbus::Result<bool>;
    fn set_pin_temporary_hidden(&self, id: &str, hidden: bool) -> zbus::Result<bool>;
    fn close_pin(&self, id: &str) -> zbus::Result<bool>;
    fn toggle_hide_all(&self) -> zbus::Result<bool>;
    fn unghost_all(&self) -> zbus::Result<u32>;
    fn list_active_pins(&self) -> zbus::Result<String>;
    fn get_clipboard_image_scale(&self) -> zbus::Result<f64>;
    fn set_clipboard_png(&self, input_fd: Fd<'_>, length: u32) -> zbus::Result<()>;
    fn capture_screenshot(&self, output_fd: Fd<'_>) -> zbus::Result<f64>;
    fn choose_capture(
        &self,
        output_fd: Fd<'_>,
        default_action: &str,
        offered_actions: &[&str],
    ) -> zbus::Result<(f64, String, StageRegion)>;
    fn choose_screenshot(
        &self,
        output_fd: Fd<'_>,
        default_action: &str,
        offer_longshot: bool,
    ) -> zbus::Result<(f64, String, StageRegion)>;
    fn select_screenshot(
        &self,
        output_fd: Fd<'_>,
        default_action: &str,
    ) -> zbus::Result<(f64, String)>;
    fn get_screenshot_settings(&self) -> zbus::Result<(bool, String, String, bool)>;
}

/// 统一前端 D-Bus 客户端（通过 Session Bus 调用 `Frontend1` 接口）
pub struct FrontendClient {
    proxy: Frontend1Proxy<'static>,
}

impl FrontendClient {
    /// 连接到当前用户 Session Bus 并创建前端代理
    pub fn connect() -> Result<Self> {
        let connection = Connection::session().map_err(map_zbus_err)?;
        let proxy = Frontend1Proxy::new(&connection).map_err(map_zbus_err)?;

        Ok(Self { proxy })
    }

    /// 探测前端是否在线并返回前端标识（如 `"gnome-shell-51"`）
    pub fn ping(&self) -> Result<String> {
        self.proxy.ping().map_err(map_zbus_err)
    }

    /// 通知前端创建并显示一张新贴图
    pub fn create_pin(&self, payload: &PinPayload) -> Result<bool> {
        let json = serde_json::to_string(payload)?;
        self.proxy.create_pin(&json).map_err(map_zbus_err)
    }

    /// 原地更新指定贴图内容（用于标注回流等场景）
    pub fn update_pin(&self, id: &str, payload: &PinPayload) -> Result<bool> {
        let json = serde_json::to_string(payload)?;
        self.proxy.update_pin(id, &json).map_err(map_zbus_err)
    }

    /// 临时隐藏或恢复指定贴图（进入/退出 satty 标注时避让）
    pub fn set_pin_temporary_hidden(&self, id: &str, hidden: bool) -> Result<bool> {
        self.proxy
            .set_pin_temporary_hidden(id, hidden)
            .map_err(map_zbus_err)
    }

    /// 关闭指定贴图并释放前端纹理
    pub fn close_pin(&self, id: &str) -> Result<bool> {
        self.proxy.close_pin(id).map_err(map_zbus_err)
    }

    /// 一键切换全部贴图的隐藏/显示状态，返回切换后是否处于隐藏态
    pub fn toggle_hide_all(&self) -> Result<bool> {
        self.proxy.toggle_hide_all().map_err(map_zbus_err)
    }

    /// 一键解除全部贴图的点击穿透，返回解除数量
    pub fn unghost_all(&self) -> Result<u32> {
        self.proxy.unghost_all().map_err(map_zbus_err)
    }

    /// 查询前端当前活跃贴图列表
    pub fn list_active_pins(&self) -> Result<Vec<ActivePinInfo>> {
        let raw = self.proxy.list_active_pins().map_err(map_zbus_err)?;
        let items: Vec<ActivePinInfo> = serde_json::from_str(&raw)?;
        Ok(items)
    }

    /// 新版扩展可返回长截图选区，旧版扩展继续返回静态截图
    pub(crate) fn select_screenshot(
        &self,
        output: &File,
        default_action: &str,
    ) -> Result<(f64, String, StageRegion)> {
        let fd = || Fd::from(output.as_fd());
        let offered_actions: &[&str] = if cfg!(feature = "longshot") {
            &["longshot", "longshot-horizontal", "gif"]
        } else {
            &[]
        };
        match self
            .proxy
            .choose_capture(fd(), default_action, offered_actions)
        {
            Err(ref error) if unknown_method(error) => {}
            result => return result.map_err(map_zbus_err),
        }
        match self
            .proxy
            .choose_screenshot(fd(), default_action, cfg!(feature = "longshot"))
        {
            Err(ref error) if unknown_method(error) => {}
            result => return result.map_err(map_zbus_err),
        }
        let empty = (0, 0, 0, 0);
        match self.proxy.select_screenshot(fd(), default_action) {
            Err(ref error) if unknown_method(error) => {
                let scale = self.proxy.capture_screenshot(fd()).map_err(map_zbus_err)?;
                Ok((scale, default_action.to_owned(), empty))
            }
            result => result
                .map(|(scale, action)| (scale, action, empty))
                .map_err(map_zbus_err),
        }
    }

    /// 交给 Shell 发布 PNG，避免普通 Wayland 剪贴板源被 Shell 扩展按错误的 MIME 读取
    pub(crate) fn set_clipboard_png(&self, png: &[u8]) -> Result<bool> {
        let length =
            u32::try_from(png.len()).map_err(|_| PinshotError::Dbus("剪贴板 PNG 太大".into()))?;
        let dir = std::env::temp_dir();
        let mut file = tempfile::tempfile().map_err(|error| PinshotError::io(&dir, error))?;
        file.write_all(png)
            .map_err(|error| PinshotError::io(&dir, error))?;
        file.rewind()
            .map_err(|error| PinshotError::io(&dir, error))?;
        match self.proxy.set_clipboard_png(Fd::from(file.as_fd()), length) {
            Ok(()) => Ok(true),
            Err(ref error) if unknown_method(error) => Ok(false),
            Err(error) => Err(map_zbus_err(error)),
        }
    }

    /// 查询外部剪贴板图片的显示倍率；旧版或未启动的扩展沿用逻辑尺寸
    pub fn clipboard_image_scale(&self) -> Result<f64> {
        match self.proxy.get_clipboard_image_scale() {
            Ok(scale) if scale.is_finite() && scale > 0.0 => Ok(scale),
            Ok(scale) => Err(PinshotError::Dbus(format!("无效的剪贴板图片倍率：{scale}"))),
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.ServiceUnknown"
                        | "org.freedesktop.DBus.Error.NameHasNoOwner"
                ) =>
            {
                Ok(1.0)
            }
            Err(err) => Err(map_zbus_err(err)),
        }
    }

    /// 查询扩展首选项设置的截图输出策略
    pub fn screenshot_settings(&self) -> Result<ScreenshotSettings> {
        let (auto_save, save_dir, filename_prefix, notify) =
            self.proxy.get_screenshot_settings().map_err(map_zbus_err)?;
        Ok(ScreenshotSettings {
            auto_save,
            save_dir,
            filename_prefix,
            notify,
        })
    }
}

pub(crate) fn unknown_method(err: &zbus::Error) -> bool {
    matches!(err, zbus::Error::MethodError(name, _, _) if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod")
}

fn map_zbus_err(err: zbus::Error) -> PinshotError {
    let text = err.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        return PinshotError::Dbus(format!(
            "前端服务 `{FRONTEND_BUS_NAME}` 未在线，请确认 GNOME Shell 扩展 `pinshot@dgxz99` 已启用 ({text})"
        ));
    }
    PinshotError::Dbus(text)
}

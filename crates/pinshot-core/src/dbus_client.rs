use std::fs::File;
use std::os::fd::AsFd;

use zbus::blocking::Connection;
use zbus::zvariant::Fd;

use crate::capture::ScreenshotSettings;
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
    fn set_system_hidden(&self, hidden: bool) -> zbus::Result<bool>;
    fn unghost_all(&self) -> zbus::Result<u32>;
    fn list_active_pins(&self) -> zbus::Result<String>;
    fn capture_screenshot(&self, output_fd: Fd<'_>) -> zbus::Result<f64>;
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

    /// 系统流程（如长图录制）临时隐藏或还原已有贴图
    pub fn set_system_hidden(&self, hidden: bool) -> Result<bool> {
        self.proxy.set_system_hidden(hidden).map_err(map_zbus_err)
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

    /// 返回用户确认的动作和 PNG 缩放；旧版扩展仅支持入口指定的动作
    pub(crate) fn select_screenshot(
        &self,
        output: &File,
        default_action: &str,
    ) -> Result<(f64, String)> {
        let fd = Fd::from(output.as_fd());
        let selection = self.proxy.select_screenshot(fd, default_action);
        match selection {
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.UnknownMethod" =>
            {
                let scale = self
                    .proxy
                    .capture_screenshot(Fd::from(output.as_fd()))
                    .map_err(map_zbus_err)?;
                Ok((scale, default_action.to_owned()))
            }
            result => result.map_err(map_zbus_err),
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

fn map_zbus_err(err: zbus::Error) -> PinshotError {
    let text = err.to_string();
    if text.contains("ServiceUnknown") || text.contains("NameHasNoOwner") {
        return PinshotError::Dbus(format!(
            "前端服务 `{FRONTEND_BUS_NAME}` 未在线，请确认 GNOME Shell 扩展 `pinshot@dgxz99` 已启用 ({text})"
        ));
    }
    PinshotError::Dbus(text)
}

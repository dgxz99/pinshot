use std::ffi::OsString;
use std::process::Command;

const LOGO_PATH: &str = ".local/share/gnome-shell/extensions/pinshot@dgxz99/icons/pinshot.svg";

/// 发送轻量桌面通知（优先调用 `notify-send`，不可用时降级输出到 stderr）
fn send(title: &str, body: &str, urgency: &str) {
    let mut command = Command::new("notify-send");
    command
        .arg("--app-name=pinshot")
        .arg(format!("--urgency={urgency}"))
        .arg("--expire-time=2500");
    if let Some(logo) = dirs::home_dir()
        .map(|home| home.join(LOGO_PATH))
        .filter(|path| path.is_file())
    {
        // GNOME 把 image-path 显示在通知正文旁，而非替换通知来源的小图标
        let mut hint = OsString::from("--hint=string:image-path:");
        hint.push(logo);
        command.arg(hint);
    }
    let result = command.arg(title).arg(body).status();

    match result {
        Ok(status) if status.success() => {}
        _ => {
            eprintln!("[pinshot] {title}: {body}");
        }
    }
}

/// 发送常规提示通知
pub fn info(title: &str, body: &str) {
    send(title, body, "normal");
}

/// 发送告警/错误提示通知
pub fn warn(title: &str, body: &str) {
    send(title, body, "critical");
}

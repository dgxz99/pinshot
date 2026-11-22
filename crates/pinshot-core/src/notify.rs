use std::process::Command;

/// 发送轻量桌面通知（优先调用 `notify-send`，不可用时降级输出到 stderr）
fn send(title: &str, body: &str, urgency: &str) {
    let result = Command::new("notify-send")
        .arg("--app-name=pinshot")
        .arg(format!("--urgency={urgency}"))
        .arg("--expire-time=2500")
        .arg(title)
        .arg(body)
        .status();

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

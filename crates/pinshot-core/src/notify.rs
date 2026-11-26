use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Command;

const EXT_ICON_REL: &str = "gnome-shell/extensions/pinshot@dgxz99/icons/pinshot.svg";
const HICOLOR_ICON_REL: &str = "icons/hicolor/scalable/apps/io.github.dgxz99.pinshot.svg";
const SYSTEM_LOGO_PATHS: &[&str] = &[
    "/usr/share/gnome-shell/extensions/pinshot@dgxz99/icons/pinshot.svg",
    "/usr/share/icons/hicolor/scalable/apps/io.github.dgxz99.pinshot.svg",
];

fn find_logo() -> Option<PathBuf> {
    let mut candidates = Vec::with_capacity(4);
    if let Some(data_dir) = dirs::data_local_dir() {
        candidates.push(data_dir.join(EXT_ICON_REL));
        candidates.push(data_dir.join(HICOLOR_ICON_REL));
    }
    candidates.extend(SYSTEM_LOGO_PATHS.iter().map(PathBuf::from));
    first_existing_file(&candidates)
}

fn first_existing_file(candidates: &[PathBuf]) -> Option<PathBuf> {
    candidates.iter().find(|path| path.is_file()).cloned()
}

/// 发送轻量桌面通知（优先调用 `notify-send`，不可用时降级输出到 stderr）
fn send(title: &str, body: &str, urgency: &str) {
    let mut command = Command::new("notify-send");
    command
        .arg("--app-name=pinshot")
        .arg(format!("--urgency={urgency}"))
        .arg("--expire-time=2500");
    if let Some(logo) = find_logo() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn first_existing_file_prefers_user_path_and_falls_back_to_system() {
        let dir = tempfile::tempdir().unwrap();
        let user_icon = dir.path().join("user.svg");
        let system_icon = dir.path().join("system.svg");

        assert_eq!(
            first_existing_file(&[user_icon.clone(), system_icon.clone()]),
            None
        );

        fs::write(&system_icon, "<svg/>").unwrap();
        assert_eq!(
            first_existing_file(&[user_icon.clone(), system_icon.clone()]),
            Some(system_icon.clone())
        );

        fs::write(&user_icon, "<svg/>").unwrap();
        assert_eq!(
            first_existing_file(&[user_icon.clone(), system_icon]),
            Some(user_icon)
        );
    }
}

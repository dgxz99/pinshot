use std::{fs, path::Path, process::Command};

use crate::{PinshotError, Result, capture::ensure_png_bytes};

/// 打开 Satty；未生成输出文件表示用户取消
pub fn edit_png(source: &[u8]) -> Result<Option<Vec<u8>>> {
    let temp = tempfile::tempdir().map_err(|err| PinshotError::io(std::env::temp_dir(), err))?;
    let input = temp.path().join("input.png");
    let output = temp.path().join("output.png");
    fs::write(&input, source).map_err(|err| PinshotError::io(&input, err))?;
    let status = Command::new("satty")
        .arg("--filename")
        .arg(&input)
        .arg("--output-filename")
        .arg(&output)
        .args([
            "--early-exit",
            "all",
            "--actions-on-enter",
            "save-to-file,exit",
            "--actions-on-escape",
            "exit",
        ])
        .status()
        .map_err(|err| PinshotError::ExternalCommand {
            cmd: "satty".into(),
            detail: err.to_string(),
        })?;
    if !status.success() {
        return Err(PinshotError::ExternalCommand {
            cmd: "satty".into(),
            detail: status.to_string(),
        });
    }
    read_output(&output)
}

fn read_output(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::read(path) {
        Ok(bytes) => ensure_png_bytes(&bytes).map(Some),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(PinshotError::io(path, err)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinguishes_cancel_from_invalid_output() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("output.png");
        assert!(read_output(&path).unwrap().is_none());
        fs::write(&path, b"invalid").unwrap();
        assert!(read_output(&path).is_err());
    }
}

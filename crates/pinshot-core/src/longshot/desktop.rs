use futures_util::StreamExt;
use tokio::sync::watch;

use super::recorder::Recorder;
use super::{LogicalRegion, Progress, Recording, failure, run_capture};
use crate::Result;

#[zbus::proxy(
    interface = "io.github.dgxz99.Pinshot.Frontend1",
    default_service = "io.github.dgxz99.Pinshot.Frontend",
    default_path = "/io/github/dgxz99/Pinshot/Frontend",
    gen_blocking = false
)]
trait LongshotFrontend {
    fn select_longshot(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    ) -> zbus::Result<(u32, u32, u32, u32)>;
    fn update_longshot(&self, height: u32, warning: bool) -> zbus::Result<bool>;
    #[zbus(signal)]
    fn longshot_action(&self, action: &str) -> zbus::Result<()>;
}

/// 由 GNOME 扩展框选与控制共享的那块显示器，用户确认完成后返回成品
pub fn record_desktop() -> Result<Option<Recording>> {
    run_capture(
        Some("io.github.dgxz99.Pinshot.Longshot"),
        async |connection, screen| {
            let monitor = screen.monitor;
            let (Some((x, y)), Some((width, height))) =
                (monitor.logical_position, monitor.logical_size)
            else {
                return Err(failure("定位共享显示器", "系统未返回显示器逻辑位置和尺寸"));
            };
            let frontend = LongshotFrontendProxy::new(connection)
                .await
                .map_err(|error| failure("连接 Pinshot 扩展", error))?;
            let mut actions = frontend
                .receive_longshot_action()
                .await
                .map_err(|error| failure("监听长图操作", error))?;
            let (x, y, width, height) = frontend
                .select_longshot(x, y, width, height)
                .await
                .map_err(|error| failure("框选长图区域", error))?;
            if width == 0 || height == 0 {
                return Ok(None);
            }
            let mut recorder = Recorder::new(
                monitor,
                Some(LogicalRegion {
                    x,
                    y,
                    width,
                    height,
                }),
            );
            let (sender, receiver) = watch::channel((0, false));
            // 框选与控制条由当前 D-Bus 连接持有。退出 run_capture 时，先停止采帧，
            // 再关闭 Portal 和该连接，扩展监听连接消失并恢复贴图
            tokio::select! {
                biased;
                action = actions.next() => {
                    let action = action.ok_or_else(|| failure("接收长图操作", "扩展已断开连接"))?;
                    let args = action.args().map_err(|error| failure("接收长图操作", error))?;
                    match args.action {
                        "finish" => recorder.finish().map(Some),
                        "cancel" => Ok(None),
                        action => Err(failure("接收长图操作", action)),
                    }
                }
                result = publish_progress(&frontend, receiver) => {
                    result?;
                    Err(failure("更新长图控制条", "状态通道已关闭"))
                }
                result = screen.read(None, |frame| {
                    let progress = recorder.push(frame)?;
                    sender.send_if_modified(|state| {
                        let next = match progress {
                            Progress::Started { height } | Progress::Extended { height } | Progress::LimitReached { height } => (height, false),
                            Progress::Unchanged => (state.0, false),
                            Progress::Unmatched => (state.0, true),
                        };
                        if next == *state {
                            return false;
                        }
                        *state = next;
                        true
                    });
                    Ok(progress.flow())
                }) => {
                    result?;
                    recorder.finish().map(Some)
                }
            }
        },
    )
}

async fn publish_progress(
    frontend: &LongshotFrontendProxy<'_>,
    mut states: watch::Receiver<(u32, bool)>,
) -> Result<()> {
    while states.changed().await.is_ok() {
        let (height, warning) = *states.borrow_and_update();
        let updated = frontend
            .update_longshot(height, warning)
            .await
            .map_err(|error| failure("更新长图控制条", error))?;
        if !updated {
            return Err(failure("更新长图控制条", "录制界面已关闭"));
        }
    }
    Ok(())
}

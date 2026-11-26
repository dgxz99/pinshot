use futures_util::StreamExt;
use tokio::sync::watch;

use super::recorder::Recorder;
use super::{LogicalRegion, Progress, Recording, ScreenSelection, failure, run_capture};
use crate::Result;
use crate::capture::{ScreenshotAction, ScrollAxis, StageRegion};
use crate::dbus_client::unknown_method;

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
    fn select_longshot_region(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stage_region: StageRegion,
    ) -> zbus::Result<(u32, u32, u32, u32)>;
    fn select_longshot_with_axis(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stage_region: StageRegion,
        axis: &str,
    ) -> zbus::Result<(u32, u32, u32, u32)>;
    fn update_longshot(&self, length: u32, warning: bool) -> zbus::Result<bool>;
    #[zbus(signal)]
    fn longshot_action(&self, action: &str) -> zbus::Result<()>;
}

/// 录制已授权屏幕上的选区，并返回拼接图与用户最后选择的输出动作
/// `choose_screen` 为真时，本次不恢复上次授权的显示器
pub fn record_desktop(
    choose_screen: bool,
    initial_region: Option<StageRegion>,
    axis: ScrollAxis,
) -> Result<Option<(Recording, ScreenshotAction)>> {
    let selection = ScreenSelection::from_choose(choose_screen);
    run_capture(Some(selection), async |connection, screen| {
        let monitor = screen.monitor;
        let (x, y, width, height) = screen.logical_geometry()?;
        let frontend = LongshotFrontendProxy::new(connection)
            .await
            .map_err(|error| failure("连接 Pinshot 扩展", error))?;
        let mut actions = frontend
            .receive_longshot_action()
            .await
            .map_err(|error| failure("监听长图操作", error))?;
        let selection = match (axis, initial_region) {
            (ScrollAxis::Horizontal, region) => {
                let result = frontend
                    .select_longshot_with_axis(
                        x,
                        y,
                        width,
                        height,
                        region.unwrap_or_default(),
                        "horizontal",
                    )
                    .await;
                if result.as_ref().is_err_and(unknown_method) {
                    return Err(failure(
                        "准备横向长截图",
                        "请更新 GNOME Shell 扩展并重新登录",
                    ));
                }
                result
            }
            (ScrollAxis::Vertical, Some(region)) => {
                frontend
                    .select_longshot_region(x, y, width, height, region)
                    .await
            }
            (ScrollAxis::Vertical, None) => frontend.select_longshot(x, y, width, height).await,
        };
        let (x, y, width, height) = selection.map_err(|error| failure("准备长截图选区", error))?;
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
            axis,
        );
        let (sender, receiver) = watch::channel((0, false));
        // 框选与控制条由当前 D-Bus 连接持有。退出 run_capture 时，先停止采帧，
        // 再关闭 Portal 和该连接，扩展监听连接消失并恢复贴图
        tokio::select! {
            biased;
            action = actions.next() => {
                let action = action.ok_or_else(|| failure("接收长图操作", "扩展已断开连接"))?;
                let args = action.args().map_err(|error| failure("接收长图操作", error))?;
                if args.action == "cancel" {
                    return Ok(None);
                }
                let output = if args.action == "finish" {
                    ScreenshotAction::Pin
                } else {
                    ScreenshotAction::from_wire(args.action)?
                };
                recorder.finish().map(|recording| Some((recording, output)))
            }
            result = publish_progress(&frontend, receiver) => {
                result?;
                Err(failure("更新长图控制条", "状态通道已关闭"))
            }
            result = screen.read(None, |frame| {
                let progress = recorder.push(frame)?;
                sender.send_if_modified(|state| {
                    let next = match progress {
                        Progress::Started { length } | Progress::Extended { length } | Progress::LimitReached { length } => (length, false),
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
                recorder.finish().map(|recording| Some((recording, ScreenshotAction::Pin)))
            }
        }
    })
}

async fn publish_progress(
    frontend: &LongshotFrontendProxy<'_>,
    mut states: watch::Receiver<(u32, bool)>,
) -> Result<()> {
    while states.changed().await.is_ok() {
        let (length, warning) = *states.borrow_and_update();
        let updated = frontend
            .update_longshot(length, warning)
            .await
            .map_err(|error| failure("更新长图控制条", error))?;
        if !updated {
            return Err(failure("更新长图控制条", "录制界面已关闭"));
        }
    }
    Ok(())
}

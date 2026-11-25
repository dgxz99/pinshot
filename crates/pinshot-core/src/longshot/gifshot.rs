use std::ops::ControlFlow;
use std::path::Path;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use tokio::sync::watch;

use super::gif_encoder::{GifEncoder, GifRecording, MAX_FILE_BYTES};
use super::region::PixelRegion;
use super::{LogicalRegion, ScreenSelection, failure, run_capture};
use crate::Result;
use crate::capture::StageRegion;
use crate::dbus_client::unknown_method;

const GIF_FRAME_RATE: i32 = 10;

#[zbus::proxy(
    interface = "io.github.dgxz99.Pinshot.Frontend1",
    default_service = "io.github.dgxz99.Pinshot.Frontend",
    default_path = "/io/github/dgxz99/Pinshot/Frontend",
    gen_blocking = false
)]
trait GifFrontend {
    fn select_gif(
        &self,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        stage_region: StageRegion,
    ) -> zbus::Result<(u32, u32, u32, u32)>;
    fn update_gif(&self, seconds: u32, near_limit: bool) -> zbus::Result<bool>;
    #[zbus(signal)]
    fn longshot_action(&self, action: &str) -> zbus::Result<()>;
}

/// 录制原选区或独立框选的区域，逐帧写入临时 GIF；取消不会发布文件
pub fn record_gif(
    choose_screen: bool,
    initial_region: Option<StageRegion>,
    directory: &Path,
) -> Result<Option<GifRecording>> {
    let selection = if choose_screen {
        ScreenSelection::Choose
    } else {
        ScreenSelection::Remembered
    };
    run_capture(Some(selection), async |connection, screen| {
        let (x, y, width, height) = screen.logical_geometry()?;
        let frontend = GifFrontendProxy::new(connection)
            .await
            .map_err(|error| failure("连接 Pinshot 扩展", error))?;
        let mut actions = frontend
            .receive_longshot_action()
            .await
            .map_err(|error| failure("监听 GIF 操作", error))?;
        let (x, y, width, height) = frontend
            .select_gif(x, y, width, height, initial_region.unwrap_or_default())
            .await
            .map_err(|error| {
                if unknown_method(&error) {
                    failure("准备 GIF 选区", "请更新 GNOME Shell 扩展并重新登录")
                } else {
                    failure("准备 GIF 选区", error)
                }
            })?;
        if width == 0 || height == 0 {
            return Ok(None);
        }
        std::fs::create_dir_all(directory)
            .map_err(|error| crate::PinshotError::io(directory, error))?;
        let logical_size = screen.monitor.logical_size;
        let region = LogicalRegion {
            x,
            y,
            width,
            height,
        };
        let mut crop = None;
        let mut encoder = None;
        let mut started = None;
        let (sender, mut receiver) = watch::channel(None);
        tokio::select! {
            biased;
            action = actions.next() => {
                let action = action.ok_or_else(|| failure("接收 GIF 操作", "扩展已断开连接"))?;
                let args = action.args().map_err(|error| failure("接收 GIF 操作", error))?;
                match args.action {
                    "cancel" => return Ok(None),
                    "save" => {},
                    other => return Err(failure("接收 GIF 操作", other)),
                }
            }
            result = async {
                while receiver.changed().await.is_ok() {
                    let Some((seconds, near_limit)) = *receiver.borrow_and_update() else { continue };
                    if !frontend.update_gif(seconds, near_limit).await.map_err(|error| failure("更新 GIF 工具条", error))? {
                        return Err(failure("更新 GIF 工具条", "录制界面已关闭"));
                    }
                }
                Err(failure("更新 GIF 工具条", "状态通道已关闭"))
            } => return result,
            result = screen.read_at_rate(None, GIF_FRAME_RATE, |frame| {
                let now = Instant::now();
                let area = match crop {
                    Some(area) => area,
                    None => {
                        let area = PixelRegion::from_logical(region, logical_size, frame.dimensions())?;
                        crop = Some(area);
                        area
                    }
                };
                let frame = area.crop(frame);
                let elapsed = now.duration_since(*started.get_or_insert(now));
                let encoder = match encoder.as_mut() {
                    Some(encoder) => encoder,
                    None => encoder.insert(GifEncoder::new(directory, frame.width(), frame.height(), MAX_FILE_BYTES)?),
                };
                if !encoder.push(frame, elapsed)? {
                    return Ok(ControlFlow::Break(()));
                }
                let status = Some((elapsed.as_secs() as u32, encoder.bytes() >= MAX_FILE_BYTES * 9 / 10));
                sender.send_if_modified(|current| {
                    if *current == status { return false; }
                    *current = status;
                    true
                });
                Ok(ControlFlow::Continue(()))
            }) => result?,
        }
        let encoder = encoder.ok_or_else(|| failure("完成 GIF", "还没有可保存的帧"))?;
        encoder
            .finish(started.map_or(Duration::ZERO, |start| start.elapsed()))
            .map(Some)
    })
}

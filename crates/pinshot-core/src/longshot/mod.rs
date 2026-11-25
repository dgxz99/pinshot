//! 滚动长图和 GIF 共用的屏幕共享与采帧入口

mod desktop;
mod gif_encoder;
mod gifshot;
mod recorder;
mod region;
mod stitcher;
mod stream;
mod viewport;

pub use desktop::record_desktop;
pub use gif_encoder::GifRecording;
pub use gifshot::record_gif;
pub use recorder::{Recording, record};
pub use region::LogicalRegion;
pub use stitcher::Progress;

use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::os::fd::OwnedFd;
use std::time::Duration;

use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, ResponseError, Session};
use ashpd::enumflags2::BitFlags;
use futures_util::StreamExt;
use image::RgbaImage;
use tokio::signal::unix::{SignalKind, signal};

use crate::{PinStore, PinshotError, Result};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);
const DESKTOP_CAPTURE_NAME: &str = "io.github.dgxz99.Pinshot.Longshot";
const PERSISTENCE_VERSION: u32 = 4;

#[derive(Clone, Copy)]
enum ScreenSelection {
    Remembered,
    Choose,
}

/// 单屏采帧探测结果：视频像素尺寸与 Portal 提供的逻辑显示区域
#[derive(Debug)]
pub struct ProbeReport {
    pub frame_count: NonZeroU32,
    pub pixel_size: (u32, u32),
    pub logical_size: Option<(i32, i32)>,
    pub logical_position: Option<(i32, i32)>,
}

#[derive(Clone, Copy)]
struct Monitor {
    logical_size: Option<(i32, i32)>,
    logical_position: Option<(i32, i32)>,
}

struct SharedScreen {
    monitor: Monitor,
    remote: OwnedFd,
    node_id: u32,
}

impl SharedScreen {
    fn logical_geometry(&self) -> Result<(i32, i32, i32, i32)> {
        let (Some((x, y)), Some((width, height))) =
            (self.monitor.logical_position, self.monitor.logical_size)
        else {
            return Err(failure("定位共享显示器", "系统未返回显示器逻辑位置和尺寸"));
        };
        Ok((x, y, width, height))
    }

    async fn read(
        self,
        duration: Option<Duration>,
        consume: impl FnMut(RgbaImage) -> Result<ControlFlow<()>>,
    ) -> Result<()> {
        self.read_at_rate(duration, stream::LONGSHOT_FRAME_RATE, consume)
            .await
    }

    async fn read_at_rate(
        self,
        duration: Option<Duration>,
        max_frame_rate: i32,
        consume: impl FnMut(RgbaImage) -> Result<ControlFlow<()>>,
    ) -> Result<()> {
        stream::read_pipewire(self.remote, self.node_id, duration, max_frame_rate, consume).await
    }
}

/// 发起系统共享授权并读取指定数量的帧；`None` 表示共享被取消或停止
pub fn probe(frame_count: NonZeroU32) -> Result<Option<ProbeReport>> {
    run_capture(None, async |_, screen| {
        let monitor = screen.monitor;
        let mut received = 0;
        let mut pixel_size = (0, 0);
        screen
            .read(None, |frame| {
                pixel_size = frame.dimensions();
                received += 1;
                if received == frame_count.get() {
                    return Ok(ControlFlow::Break(()));
                }
                Ok(ControlFlow::Continue(()))
            })
            .await?;
        Ok(Some(ProbeReport {
            frame_count,
            pixel_size,
            logical_size: monitor.logical_size,
            logical_position: monitor.logical_position,
        }))
    })
}

fn run_capture<T>(
    desktop_selection: Option<ScreenSelection>,
    capture: impl AsyncFnOnce(&zbus::Connection, SharedScreen) -> Result<Option<T>>,
) -> Result<Option<T>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("启动录制运行时", error))?;
    runtime.block_on(async {
        let mut interrupt =
            signal(SignalKind::interrupt()).map_err(|error| failure("监听 Ctrl+C", error))?;
        let mut terminate =
            signal(SignalKind::terminate()).map_err(|error| failure("监听进程终止", error))?;
        let stop_requested = async {
            tokio::select! {
                _ = interrupt.recv() => (),
                _ = terminate.recv() => (),
            }
        };
        tokio::pin!(stop_requested);

        // 独立连接保证即使取消发生在 CreateSession 返回之前，断开时 Portal 也能回收请求
        let connection = zbus::Connection::session()
            .await
            .map_err(|error| failure("连接会话 D-Bus", error))?;
        // 在打开授权对话框前占用录制名，避免再次触发时把选择器叠进当前录制
        let store = if desktop_selection.is_some() {
            match connection
                .request_name_with_flags(DESKTOP_CAPTURE_NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
                .await
            {
                Ok(_) => {}
                Err(zbus::Error::NameTaken) => return Ok(None),
                Err(error) => return Err(failure("预留录制会话", error)),
            }
            Some(PinStore::open_default()?)
        } else {
            None
        };
        let permission = desktop_selection.zip(store.as_ref());
        let start = async {
            let portal = Screencast::with_connection(connection.clone())
                .await
                .map_err(|error| failure("连接 ScreenCast Portal", error))?;
            let session = portal
                .create_session(CreateSessionOptions::default())
                .await
                .map_err(|error| failure("创建 ScreenCast 会话", error))?;
            Ok::<_, PinshotError>((portal, session))
        };
        let outcome = match tokio::select! {
            _ = &mut stop_requested => None,
            result = start => Some(result),
        } {
            Some(Ok((portal, session))) => {
                let result = tokio::select! {
                    _ = &mut stop_requested => Ok(None),
                    result = capture_session(&connection, &portal, &session, permission, capture) => result,
                };
                close_session(&session).await;
                result
            }
            Some(Err(error)) => Err(error),
            None => Ok(None),
        };
        if let Err(error) = connection.close().await {
            eprintln!("[pinshot] 断开录制 D-Bus 连接失败: {error}");
        }
        outcome
    })
}

async fn capture_session<T>(
    connection: &zbus::Connection,
    portal: &Screencast,
    session: &Session<Screencast>,
    permission: Option<(ScreenSelection, &PinStore)>,
    capture: impl AsyncFnOnce(&zbus::Connection, SharedScreen) -> Result<Option<T>>,
) -> Result<Option<T>> {
    let closed = session
        .receive_closed()
        .await
        .map_err(|error| failure("监听屏幕共享状态", error))?;
    tokio::pin!(closed);
    let mut options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from_flag(SourceType::Monitor))
        .set_multiple(false)
        .set_cursor_mode(CursorMode::Hidden);
    let permission = permission.filter(|_| portal.version() >= PERSISTENCE_VERSION);
    if let Some((selection, store)) = permission {
        let token = take_restore_token(selection, store)?;
        options = options
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(token.as_deref());
    }
    let selection = portal
        .select_sources(session, options)
        .await
        .map_err(|error| failure("请求单屏共享", error))?;
    if portal_response(selection.response(), "请求单屏共享")?.is_none() {
        return Ok(None);
    }
    let request = portal
        .start(session, None, StartCastOptions::default())
        .await
        .map_err(|error| failure("等待系统共享授权", error))?;
    let Some(streams) = portal_response(request.response(), "启动屏幕共享")? else {
        return Ok(None);
    };
    let [source] = streams.streams() else {
        return Err(failure("启动屏幕共享", "系统没有返回单个显示器的视频流"));
    };
    if let Some((_, store)) = permission {
        update_restore_token(store, streams.restore_token())?;
    }
    let remote = portal
        .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(|error| failure("连接已授权的 PipeWire 流", error))?;
    let screen = SharedScreen {
        monitor: Monitor {
            logical_size: source.size(),
            logical_position: source.position(),
        },
        remote,
        node_id: source.pipe_wire_node_id(),
    };
    tokio::select! {
        biased;
        signal = closed.next() => {
            if signal.is_some() {
                Ok(None)
            } else {
                Err(failure("监听屏幕共享状态", "会话信号流意外结束"))
            }
        }
        result = capture(connection, screen) => result,
    }
}

/// 令牌只交给 Portal 使用一次；主动选屏不消耗旧令牌，取消选屏时仍可恢复旧屏
fn take_restore_token(selection: ScreenSelection, store: &PinStore) -> Result<Option<String>> {
    match selection {
        ScreenSelection::Choose => Ok(None),
        ScreenSelection::Remembered => {
            let token = store.load_screencast_token()?;
            if token.is_some() {
                store.clear_screencast_token()?;
            }
            Ok(token)
        }
    }
}

/// 只有 Portal 在成功授权时返回令牌才写入；新屏未授权记住时，不回退到旧屏
fn update_restore_token(store: &PinStore, token: Option<&str>) -> Result<()> {
    if let Some(token) = token {
        store.save_screencast_token(token)
    } else {
        store.clear_screencast_token()
    }
}

fn portal_response<T>(
    response: std::result::Result<T, ashpd::Error>,
    step: &str,
) -> Result<Option<T>> {
    match response {
        Ok(value) => Ok(Some(value)),
        Err(ashpd::Error::Response(ResponseError::Cancelled)) => Ok(None),
        Err(error) => Err(failure(step, error)),
    }
}

async fn close_session(session: &Session<Screencast>) {
    match tokio::time::timeout(CLOSE_TIMEOUT, session.close()).await {
        Ok(Ok(())) => {}
        // 用户关闭共享或取消选择后，Portal 可能已撤销该会话对象
        Ok(Err(ashpd::Error::Portal(ashpd::PortalError::ZBus(zbus::Error::MethodError(
            name,
            _,
            _,
        )))))
            if matches!(
                name.as_str(),
                "org.freedesktop.DBus.Error.UnknownObject"
                    | "org.freedesktop.DBus.Error.UnknownMethod"
            ) => {}
        Ok(Err(error)) => {
            eprintln!("[pinshot] 关闭屏幕共享失败，将通过断开 D-Bus 回收会话: {error}")
        }
        Err(_) => eprintln!("[pinshot] 关闭屏幕共享超时，将通过断开 D-Bus 回收会话"),
    }
}

fn failure(step: &str, error: impl std::fmt::Display) -> PinshotError {
    PinshotError::Longshot(format!("{step}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consumes_a_cached_token_once_and_saves_the_next_grant() {
        let dir = tempfile::tempdir().unwrap();
        let store = PinStore::open_at(dir.path().to_path_buf()).unwrap();
        update_restore_token(&store, Some("first")).unwrap();
        assert_eq!(
            take_restore_token(ScreenSelection::Remembered, &store)
                .unwrap()
                .as_deref(),
            Some("first")
        );
        assert_eq!(store.load_screencast_token().unwrap(), None);
        update_restore_token(&store, Some("next")).unwrap();
        assert_eq!(
            store.load_screencast_token().unwrap().as_deref(),
            Some("next")
        );
    }

    #[test]
    fn choosing_a_screen_keeps_the_old_token_until_start_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let store = PinStore::open_at(dir.path().to_path_buf()).unwrap();
        update_restore_token(&store, Some("old")).unwrap();
        assert_eq!(
            take_restore_token(ScreenSelection::Choose, &store).unwrap(),
            None
        );
        assert_eq!(
            store.load_screencast_token().unwrap().as_deref(),
            Some("old")
        );
        update_restore_token(&store, None).unwrap();
        assert_eq!(store.load_screencast_token().unwrap(), None);
    }

    #[test]
    fn distinguishes_portal_cancellation_from_failure() {
        assert_eq!(portal_response(Ok(42), "请求").unwrap(), Some(42));
        let cancelled = Err::<(), _>(ashpd::Error::Response(ResponseError::Cancelled));
        assert_eq!(portal_response(cancelled, "请求").unwrap(), None);
        let failed = Err::<(), _>(ashpd::Error::Response(ResponseError::Other));
        assert!(portal_response(failed, "请求").is_err());
    }
}

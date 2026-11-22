//! 滚动长图的屏幕共享与采帧入口；图像只在内存中处理

mod recorder;
mod region;
mod stitcher;
mod stream;

pub use recorder::{Recording, record};
pub use region::LogicalRegion;
pub use stitcher::Progress;

use std::num::NonZeroU32;
use std::ops::ControlFlow;
use std::time::Duration;

use ashpd::desktop::screencast::{
    CursorMode, OpenPipeWireRemoteOptions, Screencast, SelectSourcesOptions, SourceType,
    StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, ResponseError, Session};
use ashpd::enumflags2::BitFlags;
use futures_util::StreamExt;
use image::RgbaImage;
use tokio::signal::unix::{SignalKind, signal};

use crate::{PinshotError, Result};

const CLOSE_TIMEOUT: Duration = Duration::from_secs(2);

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

/// 发起系统共享授权并读取指定数量的帧；`None` 表示共享被取消或停止
pub fn probe(frame_count: NonZeroU32) -> Result<Option<ProbeReport>> {
    let mut received = 0;
    let mut pixel_size = (0, 0);
    let monitor = run_capture(None, |_, frame| {
        pixel_size = frame.dimensions();
        received += 1;
        if received == frame_count.get() {
            return Ok(ControlFlow::Break(()));
        }
        Ok(ControlFlow::Continue(()))
    })?;
    Ok(monitor.map(|monitor| ProbeReport {
        frame_count,
        pixel_size,
        logical_size: monitor.logical_size,
        logical_position: monitor.logical_position,
    }))
}

fn run_capture(
    duration: Option<Duration>,
    consume: impl FnMut(Monitor, RgbaImage) -> Result<ControlFlow<()>>,
) -> Result<Option<Monitor>> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| failure("启动长图运行时", error))?;
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
                    result = capture_session(&portal, &session, duration, consume) => result,
                };
                close_session(&session).await;
                result
            }
            Some(Err(error)) => Err(error),
            None => Ok(None),
        };
        if let Err(error) = connection.close().await {
            eprintln!("[pinshot] 断开长图 D-Bus 连接失败: {error}");
        }
        outcome
    })
}

async fn capture_session(
    portal: &Screencast,
    session: &Session<Screencast>,
    duration: Option<Duration>,
    mut consume: impl FnMut(Monitor, RgbaImage) -> Result<ControlFlow<()>>,
) -> Result<Option<Monitor>> {
    let closed = session
        .receive_closed()
        .await
        .map_err(|error| failure("监听屏幕共享状态", error))?;
    tokio::pin!(closed);
    let options = SelectSourcesOptions::default()
        .set_sources(BitFlags::from_flag(SourceType::Monitor))
        .set_multiple(false)
        .set_cursor_mode(CursorMode::Hidden);
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
    let remote = portal
        .open_pipe_wire_remote(session, OpenPipeWireRemoteOptions::default())
        .await
        .map_err(|error| failure("连接已授权的 PipeWire 流", error))?;
    let monitor = Monitor {
        logical_size: source.size(),
        logical_position: source.position(),
    };
    tokio::select! {
        biased;
        signal = closed.next() => {
            return if signal.is_some() {
                Ok(None)
            } else {
                Err(failure("监听屏幕共享状态", "会话信号流意外结束"))
            };
        }
        result = stream::read_pipewire(remote, source.pipe_wire_node_id(), duration, |frame| consume(monitor, frame)) => result?,
    };
    Ok(Some(monitor))
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
    fn distinguishes_portal_cancellation_from_failure() {
        assert_eq!(portal_response(Ok(42), "请求").unwrap(), Some(42));
        let cancelled = Err::<(), _>(ashpd::Error::Response(ResponseError::Cancelled));
        assert_eq!(portal_response(cancelled, "请求").unwrap(), None);
        let failed = Err::<(), _>(ashpd::Error::Response(ResponseError::Other));
        assert!(portal_response(failed, "请求").is_err());
    }
}

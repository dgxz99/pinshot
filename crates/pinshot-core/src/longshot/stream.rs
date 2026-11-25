use std::ops::ControlFlow;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Duration;

use futures_util::StreamExt;
use gst::prelude::*;
use gst_video::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;
use image::RgbaImage;
use tokio::time::{Instant, timeout_at};

use super::failure;
use crate::Result;

const FRAME_TIMEOUT: Duration = Duration::from_secs(8);
const FIRST_FRAME_SETTLE: Duration = Duration::from_millis(500);
pub(super) const LONGSHOT_FRAME_RATE: i32 = 15;

/// 读取 Portal 授权的一个 PipeWire 节点，退出时先停止管线，再关闭原始描述符
pub(super) async fn read_pipewire(
    remote: OwnedFd,
    node_id: u32,
    duration: Option<Duration>,
    max_frame_rate: i32,
    consume: impl FnMut(RgbaImage) -> Result<ControlFlow<()>>,
) -> Result<()> {
    gst::init().map_err(|error| failure("初始化 GStreamer", error))?;
    let source = gst::ElementFactory::make("pipewiresrc")
        .property("fd", remote.as_raw_fd())
        // Portal 返回的是 node id，不是 target-object 所用的名称或 serial
        .property("path", node_id.to_string())
        .property("keepalive-time", 1000i32)
        .build()
        .map_err(|error| failure("创建 PipeWire 取帧源", error))?;
    read_frames(&source, duration, max_frame_rate, consume).await
}

async fn read_frames(
    source: &gst::Element,
    duration: Option<Duration>,
    max_frame_rate: i32,
    mut consume: impl FnMut(RgbaImage) -> Result<ControlFlow<()>>,
) -> Result<()> {
    let rate = gst::ElementFactory::make("videorate")
        .property("drop-only", true)
        .property("max-rate", max_frame_rate)
        .build()
        .map_err(|error| failure("创建 GStreamer 限帧器", error))?;
    let convert = gst::ElementFactory::make("videoconvert")
        .build()
        .map_err(|error| failure("创建 GStreamer 像素格式转换器", error))?;
    let caps = gst::Caps::builder("video/x-raw")
        .field("format", "RGBA")
        .build();
    let sink = gst_app::AppSink::builder()
        .caps(&caps)
        .max_buffers(2)
        .drop(true)
        .sync(false)
        .wait_on_eos(false)
        .build();
    let pipeline = RunningPipeline(gst::Pipeline::new());
    let elements = [source, &rate, &convert, sink.upcast_ref()];
    pipeline
        .0
        .add_many(elements)
        .map_err(|error| failure("构建 GStreamer 管线", error))?;
    gst::Element::link_many(elements).map_err(|error| failure("连接 GStreamer 管线", error))?;
    let bus = pipeline
        .0
        .bus()
        .ok_or_else(|| failure("启动 GStreamer 管线", "缺少消息总线"))?;
    let mut errors = bus.stream_filtered(&[gst::MessageType::Error]);
    let mut samples = sink.stream();
    pipeline
        .0
        .set_state(gst::State::Playing)
        .map_err(|error| failure("启动 GStreamer 管线", error))?;

    let mut dimensions = None;
    let mut accept_after = None;
    let mut finish_at = None;
    loop {
        let next_frame_at = Instant::now() + FRAME_TIMEOUT;
        let deadline = finish_at.map_or(next_frame_at, |end: Instant| end.min(next_frame_at));
        let sample = tokio::select! {
            biased;
            message = errors.next() => {
                if let Some(gst::MessageView::Error(error)) = message.as_ref().map(|msg| msg.view()) {
                    return Err(failure("GStreamer 取帧", format_args!("{} ({:?})", error.error(), error.debug())));
                }
                return Err(failure("GStreamer 取帧", "消息总线已关闭"));
            }
            sample = timeout_at(deadline, samples.next()) => {
                if finish_at.is_some_and(|end| Instant::now() >= end) {
                    return Ok(());
                }
                sample.map_err(|_| failure("GStreamer 取帧", "8 秒内未收到视频帧"))?
                    .ok_or_else(|| failure("GStreamer 取帧", "视频流提前结束"))?
            }
        };
        // PipeWire 可能已缓存选择器尚在屏幕上的首帧；先消费过渡帧，不让它成为画布起点
        let now = Instant::now();
        let ready = accept_after.get_or_insert_with(|| now + FIRST_FRAME_SETTLE);
        if now < *ready {
            continue;
        }
        let frame = rgba_frame(&sample)?;
        if dimensions.is_none() {
            dimensions = Some(frame.dimensions());
            finish_at = duration.map(|length| Instant::now() + length);
        } else if dimensions != Some(frame.dimensions()) {
            return Err(failure("GStreamer 取帧", "录制期间帧尺寸变更"));
        }
        if consume(frame)?.is_break() {
            return Ok(());
        }
    }
}

/// 依据实际视频元数据剔除每行的对齐填充，得到可直接裁剪的 RGBA 像素
fn rgba_frame(sample: &gst::Sample) -> Result<RgbaImage> {
    let caps = sample
        .caps()
        .ok_or_else(|| failure("解读视频帧", "缺少格式信息"))?;
    let info =
        gst_video::VideoInfo::from_caps(caps).map_err(|error| failure("解读视频帧格式", error))?;
    if info.format() != gst_video::VideoFormat::Rgba {
        return Err(failure("解读视频帧", "预期 RGBA 像素格式"));
    }
    let buffer = sample
        .buffer()
        .ok_or_else(|| failure("解读视频帧", "缺少数据缓冲区"))?;
    let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
        .map_err(|error| failure("映射视频帧", error))?;
    let bytes = frame
        .plane_data(0)
        .map_err(|error| failure("读取 RGBA 平面", error))?;
    copy_rgba_rows(
        bytes,
        frame.width(),
        frame.height(),
        frame.plane_stride()[0],
    )
}

fn copy_rgba_rows(bytes: &[u8], width: u32, height: u32, stride: i32) -> Result<RgbaImage> {
    let invalid = || failure("读取 RGBA 平面", "无效的帧尺寸、行跨度或数据长度");
    let row_bytes = (width as usize).checked_mul(4).ok_or_else(invalid)?;
    let rows = height as usize;
    let stride = usize::try_from(stride).map_err(|_| invalid())?;
    if row_bytes == 0 || rows == 0 || stride < row_bytes {
        return Err(invalid());
    }
    let required = (rows - 1)
        .checked_mul(stride)
        .and_then(|offset| offset.checked_add(row_bytes))
        .ok_or_else(invalid)?;
    if required > bytes.len() {
        return Err(invalid());
    }
    let mut pixels = Vec::with_capacity(row_bytes * rows);
    for row in 0..rows {
        let start = row * stride;
        pixels.extend_from_slice(&bytes[start..start + row_bytes]);
    }
    RgbaImage::from_raw(width, height, pixels).ok_or_else(invalid)
}

/// 在停止、取消和错误返回时释放 GStreamer 对 PipeWire 的引用
struct RunningPipeline(gst::Pipeline);

impl Drop for RunningPipeline {
    fn drop(&mut self) {
        if let Err(error) = self.0.set_state(gst::State::Null) {
            eprintln!("[pinshot] 停止录制采帧管线失败: {error}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_only_pixels_from_padded_rows() {
        let bytes = [1, 2, 3, 4, 90, 91, 92, 93, 5, 6, 7, 8];
        let frame = copy_rgba_rows(&bytes, 1, 2, 8).unwrap();
        assert_eq!(frame.into_raw(), [1, 2, 3, 4, 5, 6, 7, 8]);
        assert!(copy_rgba_rows(&bytes[..11], 1, 2, 8).is_err());
        assert!(copy_rgba_rows(&bytes, 2, 1, 4).is_err());
        assert!(copy_rgba_rows(&bytes, 1, 2, -8).is_err());
    }

    #[tokio::test]
    async fn reads_continuous_frames_and_releases_source() {
        gst::init().unwrap();
        let source = gst::ElementFactory::make("videotestsrc")
            .property("is-live", true)
            .build()
            .expect("长图测试需要 GStreamer videotestsrc 插件");
        let mut count = 0;
        read_frames(&source, None, LONGSHOT_FRAME_RATE, |frame| {
            assert_ne!(frame.dimensions(), (0, 0));
            count += 1;
            if count == 3 {
                return Ok(ControlFlow::Break(()));
            }
            Ok(ControlFlow::Continue(()))
        })
        .await
        .unwrap();
        assert_eq!(count, 3);
        assert_eq!(source.current_state(), gst::State::Null);
    }

    #[tokio::test]
    async fn dropping_pending_capture_stops_pipeline() {
        gst::init().unwrap();
        let source = gst_app::AppSrc::builder().is_live(true).build();
        let mut capture = Box::pin(read_frames(
            source.upcast_ref(),
            None,
            LONGSHOT_FRAME_RATE,
            |_| Ok(ControlFlow::Continue(())),
        ));
        assert!(futures_util::poll!(&mut capture).is_pending());
        assert_ne!(source.current_state(), gst::State::Null);
        drop(capture);
        assert_eq!(source.current_state(), gst::State::Null);
    }
}

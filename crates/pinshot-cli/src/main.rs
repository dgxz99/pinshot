use std::collections::HashSet;
use std::fs;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};

use pinshot_core::annotate;
#[cfg(feature = "longshot")]
use pinshot_core::capture::ScrollAxis;
use pinshot_core::capture::{self, CapturedImage, ScreenshotAction, ScreenshotSelection};
use pinshot_core::clipboard::{self, ClipboardContent};
use pinshot_core::protocol::PinKind;
use pinshot_core::store::NewEntryRequest;
use pinshot_core::text_render;
use pinshot_core::{Config, FrontendClient, PinStore, notify};

#[derive(Debug, Parser)]
#[command(
    name = "pinshot",
    version,
    about = "轻量级 Wayland 截图和非窗口贴图工具"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// 探测前端 D-Bus 服务在线状态
    Ping,
    /// 显示当前配置路径、缓存统计与前端连接状态
    Status,
    /// 打开区域截图工具条，默认复制截图
    Capture,
    /// 区域截屏，默认贴图
    CapturePin,
    /// 区域截屏并在 Satty 标注，确认后复制图片
    CaptureEdit,
    /// 框选并滚动拼接，完成时可复制、保存、贴图或标注
    #[cfg(feature = "longshot")]
    Longshot {
        /// 本次不使用记住的屏幕，重新打开系统选择器
        #[arg(long)]
        choose_screen: bool,
        /// 沿水平方向拼接；省略时沿纵向拼接
        #[arg(long)]
        horizontal: bool,
    },
    /// 录制一块屏幕区域并保存为动画 GIF
    #[cfg(feature = "longshot")]
    Gif {
        /// 本次不使用记住的屏幕，重新打开系统选择器
        #[arg(long)]
        choose_screen: bool,
    },
    /// 将剪贴板图片、本地图片文件或纯文本渲染并贴图
    PinClipboard,
    /// 一键切换全部贴图的隐藏/显示状态
    ToggleHide,
    /// 一键解除全部贴图的点击穿透状态
    UnghostAll,
    /// 恢复最近关闭（或上次会话遗留）的一张贴图
    RestoreLast,
    /// 清空历史缓存（默认保留活跃贴图；--all 同时关闭并清空活跃贴图）
    ClearHistory {
        /// 连同当前屏幕上的活跃贴图一并关闭并物理删除
        #[arg(long)]
        all: bool,
    },
    /// 供前端按需回调的内部短命子命令
    #[command(hide = true)]
    Internal {
        #[command(subcommand)]
        action: InternalCommands,
    },
}

#[derive(Debug, Subcommand)]
enum InternalCommands {
    /// 验证单屏共享与连续采帧，不保存截图或生成长图
    #[cfg(feature = "longshot")]
    ProbeLongshot {
        /// 读取帧数（1–60）
        #[arg(long, default_value_t = 12, value_parser = clap::value_parser!(u32).range(1..=60))]
        frames: u32,
    },
    /// 实验性定时滚动截图，导出单个 PNG
    #[cfg(feature = "longshot")]
    RecordLongshot {
        /// 所选显示器内的逻辑坐标 x,y,宽,高；省略时使用整屏
        #[arg(long)]
        region: Option<pinshot_core::longshot::LogicalRegion>,
        /// 沿水平方向拼接；省略时沿纵向拼接
        #[arg(long)]
        horizontal: bool,
        /// 从首帧开始计时（1–120 秒）
        #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u16).range(1..=120))]
        seconds: u16,
        /// PNG 保存路径（不覆盖已有文件）
        #[arg(long)]
        output: std::path::PathBuf,
    },
    /// 将指定贴图原图写入 Wayland 剪贴板
    CopyImage { id: String },
    /// 将文字贴图对应的原始纯文本写入剪贴板
    CopyText { id: String },
    /// 将指定贴图 100% 原图保存到用户配置的 save_dir
    SaveImage { id: String },
    /// 调用 satty 对指定贴图进行标注回流
    EditSatty { id: String },
    /// 将贴图在历史索引中标记为已关闭
    MarkClosed { id: String },
    /// 输出前端渲染所需的 `[pin]` 配置 JSON
    GetPinConfig,
    /// 若配置开启 restore_on_startup，则恢复上次活跃贴图
    RestoreStartup,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Ping => cmd_ping(),
        Commands::Status => cmd_status(),
        Commands::Capture => cmd_capture(ScreenshotAction::Copy),
        Commands::CapturePin => cmd_capture(ScreenshotAction::Pin),
        Commands::CaptureEdit => cmd_capture(ScreenshotAction::Edit),
        #[cfg(feature = "longshot")]
        Commands::Longshot {
            choose_screen,
            horizontal,
        } => cmd_longshot(choose_screen, None, scroll_axis(horizontal)),
        #[cfg(feature = "longshot")]
        Commands::Gif { choose_screen } => cmd_gif(choose_screen, None),
        Commands::PinClipboard => cmd_pin_clipboard(),
        Commands::ToggleHide => cmd_toggle_hide(),
        Commands::UnghostAll => cmd_unghost_all(),
        Commands::RestoreLast => cmd_restore_last(),
        Commands::ClearHistory { all } => cmd_clear_history(all),
        Commands::Internal { action } => handle_internal(action),
    }
}

fn cmd_ping() -> Result<()> {
    let client = FrontendClient::connect()?;
    let frontend_id = client.ping()?;
    println!("{frontend_id}");
    Ok(())
}

fn cmd_status() -> Result<()> {
    let config_path = Config::default_config_path()?;
    let cfg = Config::load_or_init()?;
    let store = PinStore::open_default()?;
    let history_count = store.history_count()?;

    println!("配置文件: {}", config_path.display());
    println!("缓存目录: {}", store.root_dir().display());
    println!("保存目录: {}", cfg.resolved_save_dir()?.display());
    println!("历史条目: {history_count}");

    match FrontendClient::connect().and_then(|c| {
        let id = c.ping()?;
        let active = c.list_active_pins().unwrap_or_default();
        Ok((id, active))
    }) {
        Ok((id, active)) => {
            println!("前端状态: 在线 ({id})");
            println!("活跃贴图: {}", active.len());
        }
        Err(err) => {
            println!("前端状态: 离线 ({err})");
        }
    }

    Ok(())
}

fn cmd_capture(default_action: ScreenshotAction) -> Result<()> {
    let frontend = FrontendClient::connect()?;
    let captured = match capture::capture_interactive_region(&frontend, default_action)? {
        None => return Ok(()),
        Some(ScreenshotSelection::Image(captured)) => captured,
        #[cfg(feature = "longshot")]
        Some(ScreenshotSelection::Longshot(region, axis)) => {
            return cmd_longshot(false, Some(region), axis);
        }
        #[cfg(feature = "longshot")]
        Some(ScreenshotSelection::Gif(region)) => return cmd_gif(false, Some(region)),
        #[cfg(not(feature = "longshot"))]
        Some(ScreenshotSelection::Longshot(..) | ScreenshotSelection::Gif(_)) => {
            anyhow::bail!("此版本未启用滚动长截图或 GIF 录制")
        }
    };
    let action = captured.action;
    let result = deliver_capture(&frontend, captured, PinKind::Screenshot);
    if action == ScreenshotAction::Edit
        && let Err(error) = &result
    {
        notify::warn("Pinshot 标注失败", &format!("{error:#}"));
    }
    result
}

fn deliver_capture(
    frontend: &FrontendClient,
    captured: CapturedImage,
    kind: PinKind,
) -> Result<()> {
    match captured.action {
        ScreenshotAction::Copy => Ok(clipboard::copy_png_bytes(
            frontend,
            &captured.png_bytes,
            captured.source_scale,
        )?),
        ScreenshotAction::Pin => pin_screenshot(frontend, captured, kind),
        ScreenshotAction::Edit => edit_screenshot(frontend, &captured),
        ScreenshotAction::Save => save_screenshot(
            frontend,
            &Config::load_or_init()?,
            captured.png_bytes.as_slice(),
        ),
    }
}

fn pin_screenshot(frontend: &FrontendClient, captured: CapturedImage, kind: PinKind) -> Result<()> {
    let (title, message) = if kind == PinKind::Longshot {
        ("滚动长截图", "已完成长截图并贴图")
    } else {
        ("区域截图", "已截图并贴图")
    };
    let cfg = Config::load_or_init()?;
    let output = frontend.screenshot_settings()?;
    let store = PinStore::open_default()?;
    let record = store.add_png_entry(
        NewEntryRequest {
            kind,
            png_bytes: &captured.png_bytes,
            text_content: None,
            title_preview: title.to_string(),
            source_scale: captured.source_scale,
            active: true,
        },
        &cfg.cache,
    )?;

    let mut payload = record.to_payload();
    if let Some((x, y)) = captured.initial_position {
        payload.initial_x = Some(x);
        payload.initial_y = Some(y);
    }
    push_pin_to_frontend(&store, &record.id, &payload)?;
    if let Err(error) =
        clipboard::copy_png_bytes(frontend, &captured.png_bytes, captured.source_scale)
    {
        notify::warn("Pinshot", &format!("截图已贴出，但复制失败：{error}"));
    }
    finish_screenshot(&cfg, &output, &captured.png_bytes, message)
}

fn save_screenshot(frontend: &FrontendClient, cfg: &Config, png: impl std::io::Read) -> Result<()> {
    let saved = frontend.screenshot_settings()?.export_png(cfg, png)?;
    if cfg.general.notify_on_save {
        notify::info("Pinshot 已保存原图", &saved.to_string_lossy());
    }
    println!("{}", saved.display());
    Ok(())
}

fn edit_screenshot(frontend: &FrontendClient, captured: &CapturedImage) -> Result<()> {
    if let Some(edited) = annotate::edit_png(&captured.png_bytes)? {
        let cfg = Config::load_or_init()?;
        let output = frontend.screenshot_settings()?;
        clipboard::copy_png_bytes(frontend, &edited, captured.source_scale)?;
        finish_screenshot(&cfg, &output, &edited, "已复制标注图片到剪贴板")?;
    }
    Ok(())
}

#[cfg(feature = "longshot")]
fn cmd_gif(choose_screen: bool, initial_region: Option<capture::StageRegion>) -> Result<()> {
    gif(choose_screen, initial_region).inspect_err(|error| {
        notify::warn("Pinshot", &format!("GIF 录制失败：{error}"));
    })
}

#[cfg(feature = "longshot")]
fn gif(choose_screen: bool, initial_region: Option<capture::StageRegion>) -> Result<()> {
    let output = FrontendClient::connect()?.screenshot_settings()?;
    let directory = output.export_dir(&Config::load_or_init()?)?;
    output.gif_stem()?;
    let Some(recording) =
        pinshot_core::longshot::record_gif(choose_screen, initial_region, &directory)?
    else {
        return Ok(());
    };
    let limit_reached = recording.limit_reached();
    let path = recording.save(&directory, &output.gif_stem()?)?;
    let message = if limit_reached {
        "GIF 达到大小上限，已将录制内容保存到："
    } else {
        "GIF 已保存到："
    };
    notify::info("Pinshot", &format!("{message}\n{}", path.display()));
    println!("{}", path.display());
    Ok(())
}

#[cfg(feature = "longshot")]
fn scroll_axis(horizontal: bool) -> ScrollAxis {
    if horizontal {
        ScrollAxis::Horizontal
    } else {
        ScrollAxis::Vertical
    }
}

#[cfg(feature = "longshot")]
fn cmd_longshot(
    choose_screen: bool,
    initial_region: Option<capture::StageRegion>,
    axis: ScrollAxis,
) -> Result<()> {
    longshot(choose_screen, initial_region, axis).inspect_err(|error| {
        notify::warn("Pinshot 长截图", &format!("{error:#}"));
    })
}

#[cfg(feature = "longshot")]
fn longshot(
    choose_screen: bool,
    initial_region: Option<capture::StageRegion>,
    axis: ScrollAxis,
) -> Result<()> {
    let frontend = FrontendClient::connect()?;
    let Some((recording, action)) =
        pinshot_core::longshot::record_desktop(choose_screen, initial_region, axis)?
    else {
        return Ok(());
    };
    let captured = CapturedImage {
        action,
        png_bytes: recording.png_bytes()?,
        source_scale: recording.source_scale,
        initial_position: None,
    };
    drop(recording);
    deliver_capture(&frontend, captured, PinKind::Longshot)
}

fn finish_screenshot(
    cfg: &Config,
    output: &capture::ScreenshotSettings,
    png: &[u8],
    message: &str,
) -> Result<()> {
    let saved = if output.auto_save {
        Some(output.export_png(cfg, png)?)
    } else {
        None
    };
    if output.notify {
        let body = saved.map_or_else(
            || message.to_string(),
            |path| format!("{message}\n{}", path.display()),
        );
        notify::info("Pinshot", &body);
    }
    Ok(())
}

fn edit_existing_pin(id: &str) -> Result<()> {
    let config = Config::load_or_init()?;
    let store = PinStore::open_default()?;
    let record = store
        .get(id)?
        .with_context(|| format!("未找到贴图: {id}"))?;
    let source = fs::read(&record.image_path)
        .with_context(|| format!("读取原图失败: {}", record.image_path.display()))?;
    let frontend = FrontendClient::connect()?;
    anyhow::ensure!(
        frontend.set_pin_temporary_hidden(id, true)?,
        "无法隐藏贴图: {id}"
    );

    let edit_result = (|| -> Result<()> {
        if let Some(edited) = annotate::edit_png(&source)? {
            let updated = store.replace_entry_image(id, &edited, &config.cache)?;
            anyhow::ensure!(
                frontend.update_pin(id, &updated.to_payload())?,
                "前端拒绝更新贴图: {id}"
            );
        }
        Ok(())
    })();
    let restore_result = match frontend.set_pin_temporary_hidden(id, false) {
        Ok(true) => Ok(()),
        Ok(false) => Err(anyhow::anyhow!("无法恢复贴图显示: {id}")),
        Err(err) => Err(err.into()),
    };
    match (edit_result, restore_result) {
        (Err(edit), Err(restore)) => Err(edit.context(format!("恢复贴图显示也失败: {restore:#}"))),
        (Err(err), _) | (_, Err(err)) => Err(err),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn cmd_pin_clipboard() -> Result<()> {
    let cfg = Config::load_or_init()?;
    let store = PinStore::open_default()?;

    let record = match clipboard::read_clipboard()? {
        ClipboardContent::Empty => {
            notify::info("pinshot", "剪贴板为空或不包含支持贴图的图片与文本");
            return Ok(());
        }
        ClipboardContent::Image {
            png_bytes,
            title_preview,
            source_scale,
        } => {
            let source_scale = match source_scale {
                Some(scale) => scale,
                None => FrontendClient::connect()?.clipboard_image_scale()?,
            };
            store.add_png_entry(
                NewEntryRequest {
                    kind: PinKind::ClipboardImage,
                    png_bytes: &png_bytes,
                    text_content: None,
                    title_preview,
                    source_scale,
                    active: true,
                },
                &cfg.cache,
            )?
        }
        ClipboardContent::Text(raw_text) => {
            let rendered = text_render::render_text_card(&raw_text, &cfg.text_card)?;
            store.add_png_entry(
                NewEntryRequest {
                    kind: PinKind::ClipboardText,
                    png_bytes: &rendered.png_bytes,
                    text_content: Some(&raw_text),
                    title_preview: rendered.title_preview,
                    source_scale: rendered.source_scale,
                    active: true,
                },
                &cfg.cache,
            )?
        }
    };

    push_pin_to_frontend(&store, &record.id, &record.to_payload())?;
    Ok(())
}

/// 将已入库记录推送到前端展示；若推送失败则降级为历史条目防丢失并提示用户
fn push_pin_to_frontend(
    store: &PinStore,
    record_id: &str,
    payload: &pinshot_core::protocol::PinPayload,
) -> Result<()> {
    let result = FrontendClient::connect().and_then(|client| {
        let ok = client.create_pin(payload)?;
        if !ok {
            return Err(pinshot_core::PinshotError::Dbus(
                "前端拒绝创建贴图".to_string(),
            ));
        }
        Ok(())
    });

    if let Err(err) = result {
        let _ = store.mark_closed(record_id);
        notify::warn("pinshot 贴图失败", &err.to_string());
        return Err(err.into());
    }

    Ok(())
}

fn cmd_toggle_hide() -> Result<()> {
    let client = FrontendClient::connect()?;
    let hidden = client.toggle_hide_all()?;
    if hidden {
        println!("已隐藏全部贴图");
    } else {
        println!("已恢复显示全部贴图");
    }
    Ok(())
}

fn cmd_unghost_all() -> Result<()> {
    let client = FrontendClient::connect()?;
    let count = client.unghost_all()?;
    println!("已解除 {count} 张贴图的点击穿透");
    Ok(())
}

fn cmd_restore_last() -> Result<()> {
    let store = PinStore::open_default()?;
    let client = FrontendClient::connect()?;
    let visible_ids: HashSet<String> = client
        .list_active_pins()?
        .into_iter()
        .map(|p| p.id)
        .collect();

    let Some(record) = store.latest_restorable(&visible_ids)? else {
        notify::info("pinshot", "没有可恢复的历史贴图");
        return Ok(());
    };

    if client.create_pin(&record.to_payload())? {
        store.mark_active(&record.id)?;
    }
    Ok(())
}

fn cmd_clear_history(all: bool) -> Result<()> {
    let store = PinStore::open_default()?;
    let (removed, active_ids) = store.clear_history(all)?;

    if all
        && !active_ids.is_empty()
        && let Ok(client) = FrontendClient::connect()
    {
        for id in active_ids {
            let _ = client.close_pin(&id);
        }
    }

    println!("已清理 {removed} 条缓存记录");
    Ok(())
}

#[cfg(feature = "longshot")]
fn cmd_probe_longshot(frames: u32) -> Result<()> {
    let frames = std::num::NonZeroU32::new(frames).context("采帧数量必须大于零")?;
    eprintln!("请在系统共享选择器中选择一个显示器；按 Ctrl+C 可取消");
    let Some(report) = pinshot_core::longshot::probe(frames)? else {
        println!("屏幕共享已取消或停止");
        return Ok(());
    };
    let (width, height) = report.pixel_size;
    println!(
        "已读取 {} 帧，视频尺寸: {width}×{height} 像素",
        report.frame_count
    );
    if let Some((width, height)) = report.logical_size {
        println!("显示器逻辑尺寸: {width}×{height}");
    }
    if let Some((x, y)) = report.logical_position {
        println!("显示器逻辑位置: ({x}, {y})");
    }
    Ok(())
}

#[cfg(feature = "longshot")]
fn cmd_record_longshot(
    region: Option<pinshot_core::longshot::LogicalRegion>,
    axis: ScrollAxis,
    seconds: u16,
    output: &std::path::Path,
) -> Result<()> {
    use pinshot_core::longshot::{Progress, record};
    let duration = std::time::Duration::from_secs(seconds.into());
    anyhow::ensure!(
        !output
            .try_exists()
            .with_context(|| format!("无法检查 {}", output.display()))?,
        "输出文件已存在: {}",
        output.display()
    );
    let (direction, dimension) = match axis {
        ScrollAxis::Vertical => ("纵向", "高度"),
        ScrollAxis::Horizontal => ("横向", "宽度"),
    };
    eprintln!("请选择显示器；看到「开始滚动」后再{direction}滚动，Ctrl+C 取消");
    let mut warned = false;
    let mut extended = false;
    let recording = record(region, axis, duration, |progress| match progress {
        Progress::Started { length } => {
            eprintln!("开始滚动，首帧{dimension} {length} px，{seconds} 秒后结束");
        }
        Progress::Unmatched if !warned => {
            warned = true;
            eprintln!("未找到可靠重叠，请回到上次匹配的位置，再慢慢{direction}滚动");
        }
        Progress::Unchanged => warned = false,
        Progress::Extended { .. } => {
            warned = false;
            extended = true;
        }
        Progress::LimitReached { length } => {
            eprintln!("已达到{dimension}上限 {length} px，结束采集");
            extended = true;
        }
        Progress::Unmatched => {}
    })?;
    let Some(recording) = recording else {
        println!("滚动截图已取消，未保存文件");
        return Ok(());
    };
    let (width, height) = recording.dimensions();
    if !extended {
        eprintln!("没有追加新的内容，PNG 只包含首帧");
    }
    recording.save_png(output)?;
    println!("已保存 {width}×{height} PNG: {}", output.display());
    Ok(())
}

fn handle_internal(action: InternalCommands) -> Result<()> {
    match action {
        #[cfg(feature = "longshot")]
        InternalCommands::ProbeLongshot { frames } => cmd_probe_longshot(frames),
        #[cfg(feature = "longshot")]
        InternalCommands::RecordLongshot {
            region,
            horizontal,
            seconds,
            output,
        } => cmd_record_longshot(region, scroll_axis(horizontal), seconds, &output),
        InternalCommands::CopyImage { id } => {
            let store = PinStore::open_default()?;
            let record = store
                .get(&id)?
                .with_context(|| format!("未找到贴图记录: {id}"))?;
            Ok(clipboard::copy_png_file(
                &FrontendClient::connect()?,
                &record.image_path,
                record.source_scale,
            )?)
        }
        InternalCommands::CopyText { id } => {
            let store = PinStore::open_default()?;
            let record = store
                .get(&id)?
                .with_context(|| format!("未找到贴图记录: {id}"))?;
            let text_path = record
                .text_path
                .with_context(|| format!("贴图 {id} 不包含原始文本"))?;
            let text = fs::read_to_string(&text_path)
                .with_context(|| format!("读取原文失败: {}", text_path.display()))?;
            Ok(clipboard::copy_text(&text)?)
        }
        InternalCommands::SaveImage { id } => {
            let cfg = Config::load_or_init()?;
            let store = PinStore::open_default()?;
            let record = store
                .get(&id)?
                .with_context(|| format!("未找到贴图: {id}"))?;
            let image = fs::File::open(&record.image_path)
                .with_context(|| format!("读取原图失败: {}", record.image_path.display()))?;
            save_screenshot(&FrontendClient::connect()?, &cfg, image)
        }
        InternalCommands::EditSatty { id } => edit_existing_pin(&id).inspect_err(|error| {
            notify::warn("Pinshot 标注失败", &format!("{error:#}"));
        }),
        InternalCommands::MarkClosed { id } => {
            let store = PinStore::open_default()?;
            store.mark_closed(&id)?;
            Ok(())
        }
        InternalCommands::GetPinConfig => {
            let cfg = Config::load_or_init()?;
            println!("{}", serde_json::to_string(&cfg.pin)?);
            Ok(())
        }
        InternalCommands::RestoreStartup => {
            let cfg = Config::load_or_init()?;
            if !cfg.general.restore_on_startup {
                return Ok(());
            }
            let store = PinStore::open_default()?;
            let client = FrontendClient::connect()?;
            for record in store.list_active()? {
                let _ = client.create_pin(&record.to_payload());
            }
            Ok(())
        }
    }
}

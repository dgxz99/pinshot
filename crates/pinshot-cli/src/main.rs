use std::collections::HashSet;

use anyhow::Result;
use clap::{Parser, Subcommand};

use pinshot_core::{notify, Config, FrontendClient, PinStore};

#[derive(Debug, Parser)]
#[command(
    name = "pinshot",
    version,
    about = "轻量级 Wayland 截图、真·非窗口贴图与滚动长图工具"
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
    /// 区域截屏并直接贴图（M1 实现）
    CapturePin,
    /// 区域截屏并进入 satty 标注（M1/M2 实现）
    CaptureEdit,
    /// 将剪贴板图片、本地图片文件或纯文本渲染并贴图（M1 实现）
    PinClipboard,
    /// 启动滚动长图录制与实时拼接（M3 实现）
    Longshot,
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
    /// 彻底物理删除单条历史及其图片、缩略图、文本
    DeleteHistory { id: String },
    /// 以 JSON 格式输出最近历史条目，供顶栏菜单渲染
    ListHistory { limit: Option<usize> },
    /// 按 ID 从历史缓存重新弹出贴图
    RestorePin { id: String },
    /// 若配置开启 restore_on_startup，则恢复上次活跃贴图
    RestoreStartup,
    /// 执行自定义外部命令钩子（二期）
    RunHook { hook_name: String, id: String },
    /// 执行 AI 视觉 OCR 识别（二期）
    RunOcr { id: String },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Ping => cmd_ping(),
        Commands::Status => cmd_status(),
        Commands::CapturePin => {
            anyhow::bail!("capture-pin 将在 M1 阶段接入 Portal 截屏实现");
        }
        Commands::CaptureEdit => {
            anyhow::bail!("capture-edit 将在 M1/M2 阶段接入 Portal 与 satty 实现");
        }
        Commands::PinClipboard => {
            anyhow::bail!("pin-clipboard 将在 M1 阶段接入剪贴板与 Pango 渲染实现");
        }
        Commands::Longshot => {
            anyhow::bail!("longshot 将在 M3 阶段接入滚动长图引擎实现");
        }
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
    let history = store.list_history(usize::MAX)?;

    println!("配置文件: {}", config_path.display());
    println!("缓存目录: {}", store.root_dir().display());
    println!("保存目录: {}", cfg.resolved_save_dir()?.display());
    println!("历史条目: {}", history.len());

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

    let payload = record.to_payload(None);
    if client.create_pin(&payload)? {
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

fn handle_internal(action: InternalCommands) -> Result<()> {
    match action {
        InternalCommands::CopyImage { id } => {
            anyhow::bail!("internal copy-image ({id}) 将在 M1 阶段实现");
        }
        InternalCommands::CopyText { id } => {
            anyhow::bail!("internal copy-text ({id}) 将在 M1 阶段实现");
        }
        InternalCommands::SaveImage { id } => {
            let cfg = Config::load_or_init()?;
            let store = PinStore::open_default()?;
            let save_dir = cfg.resolved_save_dir()?;
            let saved_path = store.save_to_user_dir(&id, &save_dir)?;
            if cfg.general.notify_on_save {
                notify::info(
                    "pinshot 已保存原图",
                    &saved_path.to_string_lossy(),
                );
            }
            println!("{}", saved_path.display());
            Ok(())
        }
        InternalCommands::EditSatty { id } => {
            anyhow::bail!("internal edit-satty ({id}) 将在 M2 阶段实现");
        }
        InternalCommands::MarkClosed { id } => {
            let store = PinStore::open_default()?;
            store.mark_closed(&id)?;
            Ok(())
        }
        InternalCommands::DeleteHistory { id } => {
            let store = PinStore::open_default()?;
            store.delete_entry(&id)?;
            Ok(())
        }
        InternalCommands::ListHistory { limit } => {
            let cfg = Config::load_or_init()?;
            let store = PinStore::open_default()?;
            let effective_limit = limit.unwrap_or(cfg.cache.history_menu_limit);
            let summaries: Vec<_> = store
                .list_history(effective_limit)?
                .into_iter()
                .map(|rec| rec.to_summary())
                .collect();
            println!("{}", serde_json::to_string(&summaries)?);
            Ok(())
        }
        InternalCommands::RestorePin { id } => {
            let store = PinStore::open_default()?;
            let record = store
                .get(&id)?
                .ok_or_else(|| anyhow::anyhow!("未找到历史贴图: {id}"))?;
            let client = FrontendClient::connect()?;
            if client.create_pin(&record.to_payload(None))? {
                store.mark_active(&record.id)?;
            }
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
                let _ = client.create_pin(&record.to_payload(None));
            }
            Ok(())
        }
        InternalCommands::RunHook { hook_name, id } => {
            anyhow::bail!("internal run-hook ({hook_name}, {id}) 将在 M4 阶段实现");
        }
        InternalCommands::RunOcr { id } => {
            anyhow::bail!("internal run-ocr ({id}) 将在 M4 阶段实现");
        }
    }
}

use anyhow::Result;
use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "pinshot", version, about = "轻量级 Wayland 截图、非窗口贴图与长图工具")]
struct Cli {}

fn main() -> Result<()> {
    let _cli = Cli::parse();
    Ok(())
}

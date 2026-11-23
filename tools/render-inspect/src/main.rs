//! 扫描一个 AC 车/赛道/视觉 mod，输出兼容报告。
//!
//! ```bash
//! cargo run -p vettura-render-inspect -- "mods/F1 2008 by Cimmerian_Iter"
//! cargo run -p vettura-render-inspect -- "mods/F1 2008 by Cimmerian_Iter" --json
//! ```

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use vettura_render::ext_config::ExtConfigOptions;
use vettura_render::{InspectOptions, inspect_path};

#[derive(Debug, Parser)]
#[command(
    name = "render-inspect",
    about = "扫描 AC mod，报告材质覆盖率、ext_config 绑定与 FX 模块状态"
)]
struct Args {
    /// mod 目录（车包 / 赛道 / 皮肤 / HUD 目录都可以）
    path: PathBuf,

    /// 输出 JSON，方便 CI / 兼容矩阵消费
    #[arg(long)]
    json: bool,

    /// 最多解析多少个 KN5（大包可以调大）
    #[arg(long, default_value_t = 8)]
    max_kn5: usize,

    /// 最多解析多少个 ext_config.ini
    #[arg(long, default_value_t = 128)]
    max_ext_config: usize,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let options = InspectOptions {
        max_kn5: args.max_kn5,
        ext_config: ExtConfigOptions {
            max_files: args.max_ext_config,
            ..ExtConfigOptions::default()
        },
    };

    let report = inspect_path(&args.path, options)
        .with_context(|| format!("inspect failed: {}", args.path.display()))?;

    if args.json {
        println!("{}", report.to_json());
    } else {
        print!("{}", report.to_text());
    }
    Ok(())
}

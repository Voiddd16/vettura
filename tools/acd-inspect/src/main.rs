use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use vettura_acdata::{AcdArchive, derive_key};

/// Inspect and unpack an Assetto Corsa `data.acd`.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Path to data.acd. The key is derived from its parent folder name.
    path: PathBuf,

    /// Override the derived key.
    #[arg(long)]
    key: Option<String>,

    /// Print one decrypted file to stdout.
    #[arg(long, value_name = "NAME")]
    print: Option<String>,

    /// Extract every decrypted file into this directory.
    #[arg(long, value_name = "DIR")]
    extract: Option<PathBuf>,

    /// Hide the per-file listing.
    #[arg(long)]
    quiet: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let key = match &args.key {
        Some(key) => key.clone(),
        None => {
            let folder = args
                .path
                .parent()
                .and_then(Path::file_name)
                .and_then(|name| name.to_str())
                .with_context(|| {
                    format!("cannot derive key from {}; pass --key", args.path.display())
                })?;
            derive_key(folder)
        }
    };

    let archive = AcdArchive::from_path_with_key(&args.path, &key)
        .with_context(|| format!("failed to read {}", args.path.display()))?;

    println!("file: {}", args.path.display());
    println!("key:  {key}");
    println!("files: {}", archive.len());

    if !args.quiet {
        for entry in &archive.entries {
            println!("  {:<40} {:>8} bytes", entry.name, entry.data.len());
        }
    }

    if let Some(name) = args.print {
        let bytes = archive
            .get(&name)
            .with_context(|| format!("'{name}' is not present in the archive"))?;
        println!("\n----- {name} -----");
        print!("{}", String::from_utf8_lossy(bytes));
        if !bytes.ends_with(b"\n") {
            println!();
        }
    }

    if let Some(dir) = args.extract {
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        for entry in &archive.entries {
            let name = Path::new(&entry.name)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("unnamed");
            let path = dir.join(name);
            std::fs::write(&path, &entry.data)
                .with_context(|| format!("cannot write {}", path.display()))?;
        }
        println!("\nwrote {} files to {}", archive.len(), dir.display());
    }

    Ok(())
}

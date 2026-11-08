use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use vettura_kn5::{Kn5, NodeKind};

/// Print a summary of an Assetto Corsa `.kn5` model.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Path to the .kn5 file.
    path: PathBuf,

    /// List embedded textures.
    #[arg(long)]
    textures: bool,

    /// List materials and texture slots.
    #[arg(long)]
    materials: bool,

    /// Print the node hierarchy.
    #[arg(long)]
    nodes: bool,

    /// Maximum node depth to print.
    #[arg(long, default_value_t = 3)]
    depth: usize,

    /// Write embedded DDS textures into this directory.
    #[arg(long, value_name = "DIR")]
    extract_textures: Option<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let kn5 = Kn5::from_path(&args.path)
        .with_context(|| format!("failed to parse {}", args.path.display()))?;

    println!("file:      {}", args.path.display());
    println!("version:   {}", kn5.version);
    if let Some(extra) = kn5.header_extra {
        println!("header:    extra={extra}");
    }
    println!("textures:  {}", kn5.textures.len());
    println!("materials: {}", kn5.materials.len());
    println!(
        "nodes:     {} ({} meshes, {} triangles)",
        kn5.node_count(),
        kn5.mesh_count(),
        kn5.triangle_count()
    );

    if args.textures {
        println!("\ntextures:");
        for texture in &kn5.textures {
            let dims = match texture.dds_info() {
                Ok(dds) => format!(
                    "{}x{} {}{}",
                    dds.width,
                    dds.height,
                    dds.fourcc_str().unwrap_or("?"),
                    if dds.mipmaps > 1 {
                        format!(" ({} mips)", dds.mipmaps)
                    } else {
                        String::new()
                    }
                ),
                Err(err) => format!("not a DDS ({err})"),
            };
            println!(
                "  [{}] {:<40} {:>10} bytes  {}",
                if texture.active { "active" } else { "inactive" },
                texture.name,
                texture.data.len(),
                dims
            );
        }
    }

    if args.materials {
        println!("\nmaterials:");
        for material in &kn5.materials {
            println!(
                "  {} ({}) blend={:?} depth={:?} alpha_test={}",
                material.name,
                material.shader,
                material.blend_mode,
                material.depth_mode,
                material.alpha_tested
            );
            for property in &material.properties {
                println!("    ${} = {:.4}", property.name, property.value_a);
            }
            for sample in &material.samples {
                println!(
                    "    {}[{}] -> {}",
                    sample.slot_name, sample.slot, sample.texture
                );
            }
        }
    }

    if args.nodes {
        println!("\nnodes:");
        kn5.walk(|node, depth| {
            if depth > args.depth {
                return;
            }
            let indent = "  ".repeat(depth);
            match &node.kind {
                NodeKind::Dummy { .. } => println!("{indent}{}", node.name),
                NodeKind::Mesh(mesh) => println!(
                    "{indent}{} [mesh: {} verts, {} tris, material {}, layer {}, lod {:.0}-{:.0}, renderable={}]",
                    node.name,
                    mesh.vertices.len(),
                    mesh.triangle_count(),
                    mesh.material,
                    mesh.layer,
                    mesh.lod_in,
                    mesh.lod_out,
                    mesh.is_renderable
                ),
                NodeKind::SkinnedMesh(mesh) => println!(
                    "{indent}{} [skinned: {} verts, {} tris, {} bones, material {}, layer {}, lod {:.0}-{:.0}]",
                    node.name,
                    mesh.vertices.len(),
                    mesh.triangle_count(),
                    mesh.bones.len(),
                    mesh.material,
                    mesh.layer,
                    mesh.lod_in,
                    mesh.lod_out
                ),
            }
        });
    }

    if let Some(dir) = args.extract_textures {
        extract_textures(&kn5, &dir)?;
    }

    Ok(())
}

fn extract_textures(kn5: &Kn5, dir: &Path) -> Result<()> {
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;

    for texture in &kn5.textures {
        let name = Path::new(&texture.name)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("texture.dds");
        let path = dir.join(name);
        std::fs::write(&path, &texture.data)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }

    println!(
        "\nwrote {} textures to {}",
        kn5.textures.len(),
        dir.display()
    );
    Ok(())
}

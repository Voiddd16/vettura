use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::Parser;
use vettura_kn5::{Kn5, NodeKind, synth::SynthOptions};

/// Print a summary of an Assetto Corsa `.kn5` model.
#[derive(Debug, Parser)]
#[command(version, about)]
struct Args {
    /// Path to the .kn5 file. Omit when generating a synthetic one.
    path: Option<PathBuf>,

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

    /// Write a synthetic .kn5 (with valid DXT1 textures) here instead of
    /// inspecting an existing file. Used by the load-time probes and benches.
    #[arg(long, value_name = "PATH")]
    synth: Option<PathBuf>,

    /// Synthetic model: mesh count.
    #[arg(long, default_value_t = 128)]
    synth_meshes: usize,

    /// Synthetic model: vertices per mesh.
    #[arg(long, default_value_t = 1024)]
    synth_vertices: usize,

    /// Synthetic model: texture count.
    #[arg(long, default_value_t = 32)]
    synth_textures: usize,

    /// Synthetic model: approximate bytes per texture.
    #[arg(long, default_value_t = 65536)]
    synth_texture_bytes: usize,

    /// Write synthetic textures with KN5-style truncated mip chains
    /// (Godot has to repair them, like real car mods).
    #[arg(long, default_value_t = true)]
    synth_truncated_mips: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();

    if let Some(out) = &args.synth {
        return write_synthetic(&args, out);
    }

    let Some(path) = args.path.as_ref() else {
        anyhow::bail!("give a .kn5 path, or use --synth PATH to generate one");
    };
    let kn5 =
        Kn5::from_path(path).with_context(|| format!("failed to parse {}", path.display()))?;

    println!("file:      {}", path.display());
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

fn write_synthetic(args: &Args, out: &Path) -> Result<()> {
    let options = SynthOptions {
        meshes: args.synth_meshes,
        vertices_per_mesh: args.synth_vertices,
        textures: args.synth_textures,
        texture_bytes: args.synth_texture_bytes,
        texture_mip_levels: 8,
        truncated_texture_mips: args.synth_truncated_mips,
    };
    let bytes = vettura_kn5::synth::synthetic_kn5(options);
    std::fs::write(out, &bytes).with_context(|| format!("cannot write {}", out.display()))?;

    println!(
        "wrote {} ({}) bytes: {} meshes x {} vertices, {} textures{}",
        out.display(),
        human_bytes(bytes.len()),
        options.meshes,
        options.vertices_per_mesh,
        options.textures,
        if options.truncated_texture_mips {
            ", truncated mip chains"
        } else {
            ", full mip chains"
        }
    );
    Ok(())
}

fn human_bytes(bytes: usize) -> String {
    const KIB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KIB * KIB {
        format!("{:.1} MiB", value / (KIB * KIB))
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{bytes} B")
    }
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

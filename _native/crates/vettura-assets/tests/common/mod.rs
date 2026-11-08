//! Synthetic file-system fixtures shared by the `vettura-assets` tests.
//!
//! Hand-rolled on purpose: the crate has no dev-dependencies, and these
//! helpers only need `create_dir_all` / `write` / `symlink`. Test parallelism
//! is handled by unique directory names (pid + counter), and cleanup happens
//! best-effort on drop.
//!
//! Each integration test binary compiles this module on its own, so some
//! helpers are unused in a given binary.

#![allow(dead_code)]

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};

/// A unique temporary directory, removed when dropped.
pub struct TempDir {
    path: PathBuf,
}

impl TempDir {
    /// Create an empty directory named after `tag`.
    pub fn new(tag: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let unique = COUNTER.fetch_add(1, Ordering::Relaxed);

        let path = std::env::temp_dir().join(format!(
            "vettura-test-{}-{tag}-{}",
            std::process::id(),
            unique
        ));
        fs::create_dir_all(&path).expect("create temp dir");
        Self { path }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Write `relative` (creating parents) with the given bytes.
    pub fn write(&self, relative: &str, bytes: impl AsRef<[u8]>) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        fs::write(&path, bytes).expect("write fixture file");
        path
    }

    /// Create a directory (relative path) inside the temp dir.
    pub fn dir(&self, relative: &str) -> PathBuf {
        let path = self.path.join(relative);
        fs::create_dir_all(&path).expect("create fixture dir");
        path
    }

    /// Create a file of exactly `size` bytes (zero-filled) at `relative`.
    pub fn write_sized(&self, relative: &str, size: usize) -> PathBuf {
        self.write(relative, vec![0u8; size])
    }

    /// Create a dangling symlink at `relative` pointing at `target`.
    ///
    /// A symlink whose metadata cannot be read exercises the
    /// "unstat-able files are skipped" path in `main_model_path`.
    #[cfg(unix)]
    pub fn write_dangling_symlink(&self, relative: &str, target: &str) -> PathBuf {
        let path = self.path.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create parent dirs");
        }
        std::os::unix::fs::symlink(target, &path).expect("create symlink");
        path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Create a car folder skeleton: a `data/` directory with the given ini files
/// (`(name, contents)` pairs), so `CarFolder::open` succeeds.
pub fn car_folder(files: &[(&str, &str)]) -> TempDir {
    let temp = TempDir::new("car");
    temp.dir("data");
    for (name, contents) in files {
        temp.write(&format!("data/{name}"), contents);
    }
    temp
}

//! Unpacking downloads. All functions here block; call them from `spawn_blocking`.

use std::fs::File;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context};

/// Unpack a `.zip`, `.tar.gz`/`.tgz` or `.tar.zst` archive into `dest`.
pub fn extract(archive: &Path, dest: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dest)?;
    let name = archive
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let file = File::open(archive).with_context(|| format!("cannot open {}", archive.display()))?;
    if name.ends_with(".zip") {
        zip::ZipArchive::new(file)?.extract(dest)?;
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        tar::Archive::new(flate2::read::GzDecoder::new(file)).unpack(dest)?;
    } else if name.ends_with(".tar.zst") {
        tar::Archive::new(zstd::stream::read::Decoder::new(file)?).unpack(dest)?;
    } else {
        bail!("don't know how to unpack {name}");
    }
    Ok(())
}

/// Find a file called `name` under `root` (searching a few levels deep).
pub fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, name: &str, depth: u32) -> Option<PathBuf> {
        let entries: Vec<_> = std::fs::read_dir(dir).ok()?.flatten().collect();
        for e in &entries {
            let p = e.path();
            if p.is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
                return Some(p);
            }
        }
        if depth == 0 {
            return None;
        }
        entries
            .iter()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .find_map(|p| walk(&p, name, depth - 1))
    }
    walk(root, name, 5)
}

/// Source archives from GitHub unpack into one top folder (`ComfyUI-0.42.5/`).
/// Move that folder's contents to `dest`.
pub fn unpack_single_folder(staging: &Path, dest: &Path) -> anyhow::Result<()> {
    let dirs: Vec<PathBuf> = std::fs::read_dir(staging)?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    let [inner] = dirs.as_slice() else {
        bail!("expected one folder inside the archive, found {}", dirs.len());
    };
    if dest.exists() {
        std::fs::remove_dir_all(dest)?;
    }
    std::fs::rename(inner, dest).with_context(|| format!("cannot move files into {}", dest.display()))?;
    let _ = std::fs::remove_dir_all(staging);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("outpost-archive-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn zip_round_trip_and_find() {
        let dir = temp_dir("zip");
        let zip_path = dir.join("pkg.zip");
        {
            let mut w = zip::ZipWriter::new(File::create(&zip_path).unwrap());
            let opts = zip::write::SimpleFileOptions::default();
            w.start_file("Top-1.0/bin/tool.exe", opts).unwrap();
            w.write_all(b"hi").unwrap();
            w.finish().unwrap();
        }
        let staging = dir.join("staging");
        extract(&zip_path, &staging).unwrap();
        assert!(find_file(&staging, "TOOL.EXE").is_some());

        let dest = dir.join("app");
        unpack_single_folder(&staging, &dest).unwrap();
        assert!(dest.join("bin").join("tool.exe").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}

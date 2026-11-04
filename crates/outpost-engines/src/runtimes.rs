//! Runtimes for tools (MCP servers) on this PC: Node.js (for `npx` servers) and uv (for
//! `uvx` servers), downloaded into Outpost's engines folder when first needed, so nothing is
//! installed system-wide. A Node or uv already on the PATH is used when Outpost has none.

use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail};

use crate::versions::{exe, uv_asset, uv_url};
use crate::{archive, download};

/// Node's current long-term-support line (its newest release is taken).
pub const NODE_LINE: &str = "latest-v24.x";

fn node_asset_suffix() -> &'static str {
    if cfg!(windows) {
        "-win-x64.zip"
    } else {
        "-linux-x64.tar.gz"
    }
}

/// Where Outpost keeps Node (its folder holds `node`, and `npx`'s script).
pub fn node_dir(engines_root: &Path) -> PathBuf {
    engines_root.join("node")
}

/// Outpost's Node, if it's been downloaded.
pub fn node_bin(engines_root: &Path) -> Option<PathBuf> {
    let dir = node_dir(engines_root);
    let bin = if cfg!(windows) { dir.join("node.exe") } else { dir.join("bin").join("node") };
    bin.is_file().then_some(bin)
}

/// `npx`'s script inside Outpost's Node (run it with that Node).
pub fn npx_script(engines_root: &Path) -> Option<PathBuf> {
    let dir = node_dir(engines_root);
    [dir.join("node_modules/npm/bin/npx-cli.js"), dir.join("lib/node_modules/npm/bin/npx-cli.js")]
        .into_iter()
        .find(|p| p.is_file())
}

/// Outpost's uv, if it's been downloaded (shared with ComfyUI and voice).
pub fn uv_bin(engines_root: &Path) -> Option<PathBuf> {
    let bin = engines_root.join("uv").join("bin").join(exe("uv"));
    bin.is_file().then_some(bin)
}

/// A program on the PATH, if there is one (`npx`, `uvx`...).
pub fn on_path(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{name}.exe"), format!("{name}.cmd"), format!("{name}.bat")]
    } else {
        vec![name.to_string()]
    };
    std::env::split_paths(&path).find_map(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
}

/// Download Node into `<engines>/node` (checked against nodejs.org's checksums).
pub async fn ensure_node(client: &reqwest::Client, engines_root: &Path, on_progress: impl FnMut(u64, Option<u64>)) -> anyhow::Result<PathBuf> {
    if let Some(bin) = node_bin(engines_root) {
        return Ok(bin);
    }
    let base = format!("https://nodejs.org/dist/{NODE_LINE}");
    let sums = download::fetch_text(client, &format!("{base}/SHASUMS256.txt")).await?;
    let (hash, asset) = sums
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((p.next()?.to_string(), p.next()?.to_string()))
        })
        .find(|(_, f)| f.starts_with("node-v") && f.ends_with(node_asset_suffix()))
        .ok_or_else(|| anyhow!("nodejs.org didn't list a Node download for this PC"))?;
    let staging = engines_root.join("node-download");
    let file = staging.join(&asset);
    let got = download::download(client, &format!("{base}/{asset}"), &file, on_progress).await?;
    if !got.eq_ignore_ascii_case(&hash) {
        let _ = tokio::fs::remove_file(&file).await;
        bail!("The Node download is damaged (checksum mismatch). Please try again.");
    }
    let (archive_path, unpacked, dest) = (file.clone(), staging.join("unpacked"), node_dir(engines_root));
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if unpacked.exists() {
            std::fs::remove_dir_all(&unpacked)?;
        }
        archive::extract(&archive_path, &unpacked)?;
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        archive::unpack_single_folder(&unpacked, &dest)?;
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    node_bin(engines_root).ok_or_else(|| anyhow!("Node wasn't in its download"))
}

/// Download uv into `<engines>/uv/bin` (the same uv ComfyUI and voice use).
pub async fn ensure_uv(client: &reqwest::Client, engines_root: &Path, on_progress: impl FnMut(u64, Option<u64>)) -> anyhow::Result<PathBuf> {
    if let Some(bin) = uv_bin(engines_root) {
        return Ok(bin);
    }
    let uv_root = engines_root.join("uv");
    let asset = uv_asset();
    let file = uv_root.join("download").join(asset);
    let hash = download::download(client, &uv_url(), &file, on_progress).await?;
    if let Some(expected) = download::fetch_text(client, &format!("{}.sha256", uv_url()))
        .await
        .ok()
        .and_then(|s| download::checksum_for(&s, asset))
    {
        if !expected.eq_ignore_ascii_case(&hash) {
            let _ = tokio::fs::remove_file(&file).await;
            bail!("The uv download is damaged (checksum mismatch). Please try again.");
        }
    }
    let target = uv_root.join("bin").join(exe("uv"));
    let (archive_path, unpacked, out) = (file.clone(), uv_root.join("unpacked"), target.clone());
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if unpacked.exists() {
            std::fs::remove_dir_all(&unpacked)?;
        }
        archive::extract(&archive_path, &unpacked)?;
        // uvx comes with uv; keep both.
        for name in ["uv", "uvx"] {
            if let Some(found) = archive::find_file(&unpacked, &exe(name)) {
                let to = out.with_file_name(exe(name));
                std::fs::create_dir_all(to.parent().expect("bin dir"))?;
                std::fs::copy(found, &to)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&to, std::fs::Permissions::from_mode(0o755))?;
                }
            }
        }
        let _ = std::fs::remove_dir_all(&unpacked);
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_file(&file).await;
    if !target.is_file() {
        bail!("uv wasn't in its download");
    }
    Ok(target)
}

/// Where Outpost keeps a Java (JDK) of one version, for building in server folders.
pub fn java_dir(engines_root: &Path, version: u32) -> PathBuf {
    engines_root.join(format!("java-{version}"))
}

/// Outpost's Java of that version (its home folder), if it's been downloaded.
pub fn java_home(engines_root: &Path, version: u32) -> Option<PathBuf> {
    let dir = java_dir(engines_root, version);
    dir.join("bin").join(exe("java")).is_file().then_some(dir)
}

/// The Java version a project wants, from its build files (Gradle's toolchain or release
/// setting, Maven's release or source): 25 when it asks for 22 or newer, else 21.
pub fn java_for_project(build_files: &str) -> u32 {
    let text = build_files.to_lowercase();
    let mut wanted = 21;
    for key in ["languageversion.of(", "javalanguageversion.of(", "release = ", "release.set(", "<maven.compiler.release>", "<release>", "<java.version>", "sourcecompatibility = javaversion.version_"] {
        for (i, _) in text.match_indices(key) {
            let digits: String = text[i + key.len()..].chars().skip_while(|c| !c.is_ascii_digit()).take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(v) = digits.parse::<u32>() {
                if (22..=40).contains(&v) {
                    wanted = wanted.max(25);
                }
            }
        }
    }
    wanted
}

/// Download Eclipse Temurin's JDK `version` (21, 25...) into `<engines>/java-<version>`,
/// checked against Adoptium's checksum. Returns its home folder.
pub async fn ensure_java(client: &reqwest::Client, engines_root: &Path, version: u32, on_progress: impl FnMut(u64, Option<u64>)) -> anyhow::Result<PathBuf> {
    if let Some(home) = java_home(engines_root, version) {
        return Ok(home);
    }
    let (os, ext) = if cfg!(windows) { ("windows", ".zip") } else if cfg!(target_os = "macos") { ("mac", ".tar.gz") } else { ("linux", ".tar.gz") };
    let arch = if cfg!(target_arch = "aarch64") { "aarch64" } else { "x64" };
    let api = format!("https://api.adoptium.net/v3/assets/latest/{version}/hotspot?architecture={arch}&image_type=jdk&os={os}&vendor=eclipse");
    let listing: serde_json::Value = serde_json::from_str(&download::fetch_text(client, &api).await?)?;
    let package = listing
        .as_array()
        .and_then(|a| a.first())
        .and_then(|r| r.pointer("/binary/package"))
        .ok_or_else(|| anyhow!("Adoptium didn't list a Java {version} for this machine"))?;
    let link = package.get("link").and_then(|v| v.as_str()).ok_or_else(|| anyhow!("Adoptium sent no download link"))?;
    let sum = package.get("checksum").and_then(|v| v.as_str()).unwrap_or_default().to_string();
    let staging = engines_root.join(format!("java-{version}-download"));
    let file = staging.join(format!("jdk{ext}"));
    let got = download::download(client, link, &file, on_progress).await?;
    if !sum.is_empty() && !got.eq_ignore_ascii_case(&sum) {
        let _ = tokio::fs::remove_file(&file).await;
        bail!("The Java download is damaged (checksum mismatch). Please try again.");
    }
    let (archive_path, unpacked, dest) = (file.clone(), staging.join("unpacked"), java_dir(engines_root, version));
    tokio::task::spawn_blocking(move || -> anyhow::Result<()> {
        if unpacked.exists() {
            std::fs::remove_dir_all(&unpacked)?;
        }
        archive::extract(&archive_path, &unpacked)?;
        if dest.exists() {
            std::fs::remove_dir_all(&dest)?;
        }
        archive::unpack_single_folder(&unpacked, &dest)?;
        Ok(())
    })
    .await??;
    let _ = tokio::fs::remove_dir_all(&staging).await;
    java_home(engines_root, version).ok_or_else(|| anyhow!("Java wasn't in its download"))
}

#[cfg(test)]
mod java_tests {
    use super::java_for_project;

    #[test]
    fn the_java_a_project_wants() {
        assert_eq!(java_for_project("java { toolchain { languageVersion = JavaLanguageVersion.of(21) } }"), 21);
        assert_eq!(java_for_project("java { toolchain { languageVersion.set(JavaLanguageVersion.of(25)) } }"), 25);
        assert_eq!(java_for_project("<properties><maven.compiler.release>25</maven.compiler.release></properties>"), 25);
        assert_eq!(java_for_project("plugins { id 'java' }"), 21);
    }
}

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

/// Download `url` to `dest`, reporting `(bytes_done, total)` about four times a second.
/// Writes to `dest.part` first so a cancelled download never looks complete.
/// Returns the SHA-256 of the file, hex encoded.
pub async fn download(
    client: &reqwest::Client,
    url: &str,
    dest: &Path,
    mut on_progress: impl FnMut(u64, Option<u64>),
) -> anyhow::Result<String> {
    if let Some(dir) = dest.parent() {
        tokio::fs::create_dir_all(dir).await?;
    }
    let resp = client
        .get(url)
        .send()
        .await
        .with_context(|| format!("could not reach {url}"))?
        .error_for_status()
        .with_context(|| format!("download refused: {url}"))?;
    let total = resp.content_length();
    let part = PathBuf::from(format!("{}.part", dest.display()));
    let mut file = tokio::fs::File::create(&part)
        .await
        .with_context(|| format!("cannot write {}", part.display()))?;
    let mut hasher = Sha256::new();
    let mut done: u64 = 0;
    let mut last = Instant::now();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.context("download interrupted")?;
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        done += chunk.len() as u64;
        if last.elapsed() >= Duration::from_millis(250) {
            on_progress(done, total);
            last = Instant::now();
        }
    }
    file.flush().await?;
    drop(file);
    on_progress(done, total);
    tokio::fs::rename(&part, dest).await?;
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

pub async fn fetch_text(client: &reqwest::Client, url: &str) -> anyhow::Result<String> {
    Ok(client.get(url).send().await?.error_for_status()?.text().await?)
}

/// Find `asset`'s hash in a `sha256sum`-style listing (`<hash>  <file>` per line).
pub fn checksum_for(listing: &str, asset: &str) -> Option<String> {
    listing.lines().find_map(|line| {
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.last()?.trim_start_matches('*').trim_start_matches("./");
        (file == asset && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

/// "412 MB of 1.6 GB"
pub fn describe(done: u64, total: Option<u64>) -> String {
    match total {
        Some(t) => format!("{} of {}", human(done), human(t)),
        None => human(done),
    }
}

pub fn human(bytes: u64) -> String {
    let mb = bytes as f64 / 1_048_576.0;
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{:.0} MB", mb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_checksums() {
        let h = "a".repeat(64);
        let listing = format!(
            "{h}  ./ollama-windows-amd64.zip\n{}  ollama-linux-amd64.tar.zst\n",
            "b".repeat(64)
        );
        assert_eq!(checksum_for(&listing, "ollama-windows-amd64.zip"), Some(h));
        assert_eq!(
            checksum_for(&listing, "ollama-linux-amd64.tar.zst"),
            Some("b".repeat(64))
        );
        assert_eq!(checksum_for(&listing, "missing.zip"), None);
        let uv = format!("{} *uv-x86_64-pc-windows-msvc.zip", "C".repeat(64));
        assert_eq!(checksum_for(&uv, "uv-x86_64-pc-windows-msvc.zip"), Some("c".repeat(64)));
    }

    #[test]
    fn sizes_read_well() {
        assert_eq!(human(5 * 1_048_576), "5 MB");
        assert_eq!(describe(1_048_576 * 512, Some(1_048_576 * 2048)), "512 MB of 2.0 GB");
    }
}

//! Running child processes quietly and keeping their output.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};

use anyhow::bail;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

const KEEP_LINES: usize = 300;

/// Don't flash a console window for every engine and installer step on Windows.
pub fn hide_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    {
        // Own process group, so stopping an engine also stops its helpers.
        cmd.process_group(0);
    }
}

/// Recent output lines in memory, plus a log file on disk.
#[derive(Clone)]
pub struct Log {
    lines: Arc<Mutex<VecDeque<String>>>,
    file: PathBuf,
}

impl Log {
    pub fn new(file: PathBuf) -> Self {
        Log {
            lines: Arc::new(Mutex::new(VecDeque::new())),
            file,
        }
    }

    pub fn push(&self, line: impl Into<String>) {
        let line = line.into();
        if let Ok(mut lines) = self.lines.lock() {
            if lines.len() >= KEEP_LINES {
                lines.pop_front();
            }
            lines.push_back(line.clone());
        }
        let file = self.file.clone();
        // Best effort; a full disk should not take the engine down. Outside a Tokio
        // runtime (e.g. while the app is exiting) write synchronously instead.
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            use std::io::Write;
            if let Some(dir) = file.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&file) {
                let _ = writeln!(f, "{line}");
            }
            return;
        };
        handle.spawn(async move {
            if let Some(dir) = file.parent() {
                let _ = tokio::fs::create_dir_all(dir).await;
            }
            if let Ok(mut f) = tokio::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&file)
                .await
            {
                let _ = f.write_all(format!("{line}\n").as_bytes()).await;
            }
        });
    }

    pub fn tail(&self, n: usize) -> Vec<String> {
        self.lines
            .lock()
            .map(|l| l.iter().skip(l.len().saturating_sub(n)).cloned().collect())
            .unwrap_or_default()
    }
}

/// Copy a child's stdout and stderr into `log`, line by line.
pub fn capture_output(child: &mut Child, log: &Log) {
    fn pump(stream: impl AsyncRead + Unpin + Send + 'static, log: Log) {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let line = line.trim_end();
                if !line.is_empty() {
                    log.push(line);
                }
            }
        });
    }
    if let Some(out) = child.stdout.take() {
        pump(out, log.clone());
    }
    if let Some(err) = child.stderr.take() {
        pump(err, log.clone());
    }
}

/// Spawn hidden with piped output.
pub fn spawn(mut cmd: Command, log: &Log) -> anyhow::Result<Child> {
    hide_window(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn()?;
    capture_output(&mut child, log);
    Ok(child)
}

/// Run an installer step to completion; fail with its last output lines if it fails.
pub async fn run(cmd: Command, log: &Log) -> anyhow::Result<()> {
    let program = format!("{:?}", cmd.as_std().get_program());
    let mut child = spawn(cmd, log)?;
    let status = child.wait().await?;
    // Give the output pumps a moment to flush their last lines.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    if !status.success() {
        let tail = log.tail(6).join("\n");
        bail!("{program} failed ({status}).\n{tail}");
    }
    Ok(())
}

/// Stop a process and everything it started (Ollama runs model helpers as children).
pub fn kill_tree(child: &mut Child) {
    if let Some(pid) = child.id() {
        #[cfg(windows)]
        {
            let mut c = std::process::Command::new("taskkill");
            c.args(["/PID", &pid.to_string(), "/T", "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000);
            let _ = c.status();
        }
        #[cfg(not(windows))]
        {
            let _ = std::process::Command::new("kill")
                .args(["-TERM", &format!("-{pid}")])
                .status();
        }
    }
    let _ = child.start_kill();
}

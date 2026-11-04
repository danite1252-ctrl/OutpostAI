//! Running things in a chat's folder, for coding: one-off commands (`gradlew build`) that
//! finish, and long-running processes (a test server) that keep going until stopped. Used by
//! the desktop app for folders on a PC, and by the server for its own folders.
//!
//! A command line runs with Windows' command prompt or `sh`, in the folder, with no window.
//! On the server it can run sandboxed with bubblewrap: only its folder (and the tools it
//! needs) visible, everything else out of reach. Stopping one stops everything it started.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

/// Most of a command's output kept (its end).
const OUTPUT_KEEP: usize = 256 * 1024;
/// Most lines a process's log keeps.
const LOG_LINES: usize = 3000;
/// A process left running is stopped after this long.
const PROCESS_MAX: Duration = Duration::from_secs(3 * 60 * 60);

/// What to run, and where.
#[derive(Debug, Clone)]
pub struct Spec {
    /// Where it runs (inside the folder).
    pub dir: PathBuf,
    pub command: String,
    /// Extra environment (e.g. JAVA_HOME, PATH in front).
    pub env: Vec<(String, String)>,
    pub sandbox: Option<Sandbox>,
}

/// Run inside bubblewrap (Linux): only these places exist for it.
#[derive(Debug, Clone)]
pub struct Sandbox {
    /// The chat's folder (read and write).
    pub folder: PathBuf,
    /// Its home (caches like Gradle's; read and write).
    pub home: PathBuf,
    /// Tools it may use (read only), e.g. the Java Outpost downloaded.
    pub tools: Vec<PathBuf>,
    pub network: bool,
}

/// bubblewrap is installed and works here (user namespaces allowed). Checked once.
pub fn sandbox_available() -> bool {
    static OK: LazyLock<bool> = LazyLock::new(|| {
        cfg!(target_os = "linux")
            && std::process::Command::new("bwrap")
                .args(["--ro-bind", "/", "/", "--dev", "/dev", "--proc", "/proc", "--unshare-pid", "true"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
    });
    *OK
}

fn command_for(spec: &Spec) -> tokio::process::Command {
    #[cfg(windows)]
    let mut c = {
        let mut c = tokio::process::Command::new("cmd");
        // The command line goes to cmd as typed (quotes and all).
        c.raw_arg("/D /S /C \"").raw_arg(&spec.command).raw_arg("\"");
        // No console window flashing up.
        c.creation_flags(0x0800_0000);
        c
    };
    #[cfg(not(windows))]
    let mut c = match &spec.sandbox {
        Some(sb) => {
            let mut c = tokio::process::Command::new("bwrap");
            c.args(["--die-with-parent", "--new-session", "--unshare-pid", "--unshare-ipc", "--unshare-uts"]);
            if !sb.network {
                c.arg("--unshare-net");
            }
            for p in ["/usr", "/bin", "/sbin", "/lib", "/lib64", "/lib32", "/etc/ssl", "/etc/ca-certificates", "/etc/alternatives", "/etc/java-21-openjdk", "/run/systemd/resolve"] {
                c.args(["--ro-bind-try", p, p]);
            }
            c.args(["--ro-bind-try", "/etc/ld.so.conf.d", "/etc/ld.so.conf.d"]);
            for f in ["/etc/resolv.conf", "/etc/hosts", "/etc/passwd", "/etc/group", "/etc/nsswitch.conf", "/etc/localtime", "/etc/ld.so.cache", "/etc/ld.so.conf"] {
                c.args(["--ro-bind-try", f, f]);
            }
            c.args(["--proc", "/proc", "--dev", "/dev", "--tmpfs", "/tmp"]);
            for t in &sb.tools {
                c.arg("--ro-bind").arg(t).arg(t);
            }
            c.arg("--bind").arg(&sb.folder).arg(&sb.folder);
            c.arg("--bind").arg(&sb.home).arg(&sb.home);
            c.arg("--setenv").arg("HOME").arg(&sb.home);
            c.arg("--chdir").arg(&spec.dir);
            c.args(["sh", "-c", &spec.command]);
            c
        }
        None => {
            let mut c = tokio::process::Command::new("sh");
            c.arg("-c").arg(&spec.command);
            c
        }
    };
    #[cfg(unix)]
    {
        // Its own process group, so stopping it stops everything it started.
        c.process_group(0);
    }
    c.current_dir(&spec.dir);
    for (k, v) in &spec.env {
        c.env(k, v);
    }
    c
}

/// Stop a process and everything it started.
pub fn stop_tree(pid: Option<u32>) {
    let Some(pid) = pid else { return };
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .creation_flags(0x0800_0000)
            .status();
    }
    #[cfg(not(windows))]
    {
        // The whole group (it was started as its leader).
        let _ = std::process::Command::new("kill").args(["-9", "--", &format!("-{pid}")]).status();
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
}

// ---------------------------------------------------------------- one-off commands

/// Commands running now (their run id → process id), so Stop can end them.
static RUNNING: LazyLock<Mutex<HashMap<String, u32>>> = LazyLock::new(Default::default);

/// Run a command until it finishes (or `timeout` seconds, then it's stopped). Its output and
/// errors come back together, in order: `{exit_code, timed_out, secs, output}`.
pub async fn run(spec: &Spec, timeout: u64, run_id: Option<String>) -> Result<Value, String> {
    if spec.command.trim().is_empty() {
        return Err("Say which command to run.".into());
    }
    let mut cmd = command_for(spec);
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let started = Instant::now();
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start it: {e}"))?;
    let pid = child.id();
    if let (Some(r), Some(p)) = (&run_id, pid) {
        RUNNING.lock().unwrap_or_else(|p| p.into_inner()).insert(r.clone(), p);
    }
    let output = Arc::new(Mutex::new(Vec::<u8>::new()));
    let mut readers = Vec::new();
    let streams: Vec<Box<dyn tokio::io::AsyncRead + Unpin + Send>> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    for mut stream in streams {
        let output = output.clone();
        readers.push(tokio::spawn(async move {
            let mut buf = vec![0u8; 16 * 1024];
            while let Ok(n) = stream.read(&mut buf).await {
                if n == 0 {
                    break;
                }
                let mut o = output.lock().unwrap_or_else(|p| p.into_inner());
                o.extend_from_slice(&buf[..n]);
                if o.len() > OUTPUT_KEEP * 2 {
                    let cut = o.len() - OUTPUT_KEEP;
                    o.drain(..cut);
                }
            }
        }));
    }
    let finished = tokio::time::timeout(Duration::from_secs(timeout.clamp(10, 3600)), child.wait()).await;
    if let Some(r) = &run_id {
        RUNNING.lock().unwrap_or_else(|p| p.into_inner()).remove(r);
    }
    let (exit_code, timed_out) = match finished {
        Ok(Ok(status)) => (status.code(), false),
        Ok(Err(e)) => return Err(e.to_string()),
        Err(_) => {
            stop_tree(pid);
            (None, true)
        }
    };
    // The last of the output (something it started may keep the streams open: don't wait long).
    for r in readers {
        let _ = tokio::time::timeout(Duration::from_secs(3), r).await;
    }
    let mut output = output.lock().unwrap_or_else(|p| p.into_inner()).clone();
    if output.len() > OUTPUT_KEEP {
        let cut = output.len() - OUTPUT_KEEP;
        output.drain(..cut);
    }
    Ok(json!({
        "exit_code": exit_code,
        "timed_out": timed_out,
        "secs": started.elapsed().as_secs(),
        "output": String::from_utf8_lossy(&output),
    }))
}

/// Stop a running command (Stop was pressed).
pub fn cancel(run_id: &str) {
    let pid = RUNNING.lock().unwrap_or_else(|p| p.into_inner()).remove(run_id);
    stop_tree(pid);
}

// ---------------------------------------------------------------- long-running processes

struct Proc {
    pid: Option<u32>,
    command: String,
    log: Arc<Mutex<VecDeque<String>>>,
    /// Lines ever written (so "new since" can be asked).
    written: Arc<Mutex<u64>>,
    /// Set when it ends: its exit code (None: killed).
    ended: Arc<Mutex<Option<Option<i32>>>>,
    stdin: Arc<tokio::sync::Mutex<Option<tokio::process::ChildStdin>>>,
    started: Instant,
}

static PROCS: LazyLock<Mutex<HashMap<String, Proc>>> = LazyLock::new(Default::default);

/// Start a process that keeps running (a test server), by name. Its output is kept (the last
/// few thousand lines) for [`output`]; [`input`] types into it; [`stop`] ends it.
pub async fn start(name: &str, spec: &Spec) -> Result<Value, String> {
    {
        let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(p) = procs.get(name) {
            if p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
                return Err(format!("\"{name}\" is already running: stop it first, or read its output."));
            }
        }
    }
    let mut cmd = command_for(spec);
    cmd.stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start it: {e}"))?;
    let pid = child.id();
    let log = Arc::new(Mutex::new(VecDeque::new()));
    let written = Arc::new(Mutex::new(0u64));
    let ended = Arc::new(Mutex::new(None));
    let stdin = Arc::new(tokio::sync::Mutex::new(child.stdin.take()));
    let streams: Vec<Box<dyn tokio::io::AsyncRead + Unpin + Send>> = [
        child.stdout.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        child.stderr.take().map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
    ]
    .into_iter()
    .flatten()
    .collect();
    for stream in streams {
        let (log, written) = (log.clone(), written.clone());
        tokio::spawn(async move {
            let mut lines = tokio::io::BufReader::new(stream).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let mut l = log.lock().unwrap_or_else(|p| p.into_inner());
                l.push_back(line);
                if l.len() > LOG_LINES {
                    l.pop_front();
                }
                *written.lock().unwrap_or_else(|p| p.into_inner()) += 1;
            }
        });
    }
    {
        let ended = ended.clone();
        tokio::spawn(async move {
            // Ends by itself, or is stopped after a few hours.
            let code = match tokio::time::timeout(PROCESS_MAX, child.wait()).await {
                Ok(Ok(s)) => s.code(),
                _ => {
                    stop_tree(pid);
                    None
                }
            };
            *ended.lock().unwrap_or_else(|p| p.into_inner()) = Some(code);
        });
    }
    PROCS.lock().unwrap_or_else(|p| p.into_inner()).insert(
        name.to_string(),
        Proc { pid, command: spec.command.clone(), log, written, ended, stdin, started: Instant::now() },
    );
    // A moment for it to say something (or fail straight away).
    tokio::time::sleep(Duration::from_millis(1500)).await;
    output(name, 40, None)
}

/// A process's latest output (`lines` of it, or what's new after `after` lines written),
/// and whether it's still running: `{running, exit_code, lines, output, written, minutes}`.
pub fn output(name: &str, lines: usize, after: Option<u64>) -> Result<Value, String> {
    let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
    let p = procs.get(name).ok_or_else(|| format!("Nothing called \"{name}\" was started."))?;
    let log = p.log.lock().unwrap_or_else(|p| p.into_inner());
    let written = *p.written.lock().unwrap_or_else(|p| p.into_inner());
    let want = match after {
        Some(a) if a < written => ((written - a) as usize).min(lines.clamp(1, 400)),
        Some(_) => 0,
        None => lines.clamp(1, 400),
    };
    let shown: Vec<&str> = log.iter().rev().take(want).rev().map(String::as_str).collect();
    let ended = *p.ended.lock().unwrap_or_else(|p| p.into_inner());
    Ok(json!({
        "running": ended.is_none(),
        "exit_code": ended.flatten(),
        "command": p.command,
        "lines": shown.len(),
        "output": shown.join("\n"),
        "written": written,
        "minutes": p.started.elapsed().as_secs() / 60,
    }))
}

/// Type a line into a running process (a server's console: `stop`, `plugins`...).
pub async fn input(name: &str, text: &str) -> Result<Value, String> {
    let stdin = {
        let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
        let p = procs.get(name).ok_or_else(|| format!("Nothing called \"{name}\" was started."))?;
        if p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_some() {
            return Err(format!("\"{name}\" isn't running any more."));
        }
        p.stdin.clone()
    };
    let mut guard = stdin.lock().await;
    let s = guard.as_mut().ok_or("It doesn't take typed input.")?;
    s.write_all(format!("{}\n", text.trim_end()).as_bytes()).await.map_err(|e| e.to_string())?;
    s.flush().await.map_err(|e| e.to_string())?;
    Ok(json!({ "ok": true }))
}

/// Stop a process (and everything it started).
pub fn stop(name: &str) -> Result<Value, String> {
    let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
    let p = procs.get(name).ok_or_else(|| format!("Nothing called \"{name}\" was started."))?;
    if p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_none() {
        stop_tree(p.pid);
    }
    Ok(json!({ "ok": true }))
}

/// Processes started here whose names begin with `prefix` (a folder's): name, running.
pub fn list(prefix: &str) -> Vec<(String, bool)> {
    let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
    let mut v: Vec<(String, bool)> = procs
        .iter()
        .filter(|(n, _)| n.starts_with(prefix))
        .map(|(n, p)| (n.clone(), p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_none()))
        .collect();
    v.sort();
    v
}

/// Processes started under names beginning with `prefix`, for a list: name (without the
/// prefix), command, running, minutes since started.
pub fn describe(prefix: &str) -> Vec<Value> {
    let procs = PROCS.lock().unwrap_or_else(|p| p.into_inner());
    let mut v: Vec<Value> = procs
        .iter()
        .filter(|(n, _)| n.starts_with(prefix))
        .map(|(n, p)| {
            json!({
                "name": &n[prefix.len()..],
                "command": p.command,
                "running": p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_none(),
                "minutes": p.started.elapsed().as_secs() / 60,
            })
        })
        .collect();
    v.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    v
}

/// Commands running now, and processes still running (on this machine).
pub fn busy() -> (usize, usize) {
    let commands = RUNNING.lock().unwrap_or_else(|p| p.into_inner()).len();
    let processes = PROCS
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|p| p.ended.lock().unwrap_or_else(|p| p.into_inner()).is_none())
        .count();
    (commands, processes)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn here(command: &str) -> Spec {
        Spec { dir: std::env::temp_dir(), command: command.into(), env: vec![("GREETING".into(), "hi".into())], sandbox: None }
    }

    #[tokio::test]
    async fn a_command_runs_and_reports() {
        let v = run(&here("echo $GREETING; echo oops >&2; exit 3"), 30, None).await.unwrap();
        assert_eq!(v["exit_code"], 3);
        let out = v["output"].as_str().unwrap();
        assert!(out.contains("hi") && out.contains("oops"), "{out}");
        // Too long: stopped.
        let v = run(&here("sleep 30"), 10, None).await.unwrap();
        assert_eq!(v["timed_out"], true);
    }

    #[tokio::test]
    async fn a_process_keeps_running_takes_input_and_stops() {
        let name = format!("t-{}", uuid::Uuid::new_v4());
        start(&name, &here("echo ready; while read l; do echo got $l; done")).await.unwrap();
        input(&name, "plugins").await.unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        let o = output(&name, 10, None).unwrap();
        assert_eq!(o["running"], true);
        assert!(o["output"].as_str().unwrap().contains("got plugins"), "{o}");
        assert!(start(&name, &here("true")).await.is_err(), "one at a time");
        stop(&name).unwrap();
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(output(&name, 10, None).unwrap()["running"], false);
        assert!(list("t-").iter().any(|(n, _)| n == &name));
    }
}

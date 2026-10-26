//! Logs and crash reports, kept on this PC only (nothing is ever sent anywhere).
//!
//! - Logs: `<data>/logs/outpost.log`, rolled over at 5 MB (three old files kept), plus
//!   the last few hundred lines in memory for crash reports and the Diagnostics screen.
//! - Crash reports: `<data>/crash-reports/crash-<time>.txt`, written when Outpost panics,
//!   and (in release builds) when the last run didn't close normally (power cut, forced close,
//!   or a crash the panic hook couldn't catch).
//!
//! [`init`] sets up both; call it once at start-up, before anything else logs.

use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

const LOG_FILE: &str = "outpost.log";
const LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const LOG_KEEP: usize = 3;
const RING_LINES: usize = 400;
/// Crash reports kept; older ones are removed.
const CRASH_KEEP: usize = 50;

struct Diag {
    data_dir: PathBuf,
    app: &'static str,
    file: Mutex<Option<File>>,
    ring: Mutex<VecDeque<String>>,
}

static DIAG: OnceLock<Diag> = OnceLock::new();

pub fn logs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn crash_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("crash-reports")
}

/// Logging to the console and `<data>/logs`, a panic hook that writes crash reports, and
/// a note if the last run of `app` ("desktop" or "gateway") didn't close normally.
pub fn init(data_dir: &Path, app: &'static str, default_filter: &str) {
    let _ = fs::create_dir_all(logs_dir(data_dir));
    let _ = fs::create_dir_all(crash_dir(data_dir));
    let diag = DIAG.get_or_init(|| Diag {
        data_dir: data_dir.to_path_buf(),
        app,
        file: Mutex::new(open_log(data_dir)),
        ring: Mutex::new(VecDeque::with_capacity(RING_LINES)),
    });

    let filter = || EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_filter));
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter()))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Sink)
                .with_filter(filter()),
        )
        .try_init();

    // Before the hook: an unclean previous run is reported with the old log's last lines.
    check_last_run(diag);

    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let what = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(no message)".to_string());
        let place = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let body = format!(
            "What: {what}\nWhere: {place}\nThread: {thread}\n\nBacktrace:\n{backtrace}\n\nLast log lines:\n{}",
            recent_log(200)
        );
        write_report("crash", "Outpost hit an error it couldn't recover from", &body);
        previous(info);
    }));
}

/// Call when Outpost closes normally, so the next start doesn't report an unclean exit.
pub fn clean_exit() {
    if let Some(d) = DIAG.get() {
        let _ = fs::remove_file(running_marker(&d.data_dir, d.app));
    }
}

fn running_marker(data_dir: &Path, app: &str) -> PathBuf {
    logs_dir(data_dir).join(format!("running-{app}"))
}

fn check_last_run(d: &Diag) {
    let marker = running_marker(&d.data_dir, d.app);
    // Development builds are stopped with Ctrl+C all the time: only installed builds report this.
    if !cfg!(debug_assertions) && marker.exists() {
        let started = fs::read_to_string(&marker).unwrap_or_default();
        let tail = tail_of(&logs_dir(&d.data_dir).join(LOG_FILE), 200);
        let body = format!(
            "What: Outpost ({}) didn't close normally last time.\nIt may have been forced to close, the PC lost power, or it crashed in a way it couldn't report.\nThat run started: {}\n\nLast log lines from that run:\n{tail}",
            d.app,
            started.trim()
        );
        write_report("unclean", "Outpost didn't close normally last time", &body);
    }
    let _ = fs::write(&marker, rfc3339(outpost_core::now()));
}

fn open_log(data_dir: &Path) -> Option<File> {
    let path = logs_dir(data_dir).join(LOG_FILE);
    if fs::metadata(&path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false) {
        roll(data_dir);
    }
    OpenOptions::new().create(true).append(true).open(path).ok()
}

/// outpost.log → outpost.1.log → outpost.2.log …, dropping the oldest.
fn roll(data_dir: &Path) {
    let dir = logs_dir(data_dir);
    let _ = fs::remove_file(dir.join(format!("outpost.{LOG_KEEP}.log")));
    for n in (1..LOG_KEEP).rev() {
        let _ = fs::rename(dir.join(format!("outpost.{n}.log")), dir.join(format!("outpost.{}.log", n + 1)));
    }
    let _ = fs::rename(dir.join(LOG_FILE), dir.join("outpost.1.log"));
}

/// Writes formatted log lines to the log file and the in-memory ring.
#[derive(Clone, Copy)]
struct Sink;

impl<'a> MakeWriter<'a> for Sink {
    type Writer = Sink;
    fn make_writer(&'a self) -> Self::Writer {
        Sink
    }
}

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let Some(d) = DIAG.get() else { return Ok(buf.len()) };
        if let Ok(mut file) = d.file.lock() {
            let too_big = file.as_ref().and_then(|f| f.metadata().ok()).is_some_and(|m| m.len() > LOG_MAX_BYTES);
            if too_big {
                *file = None;
                roll(&d.data_dir);
                *file = open_log(&d.data_dir);
            }
            if let Some(f) = file.as_mut() {
                let _ = f.write_all(buf);
            }
        }
        if let Ok(mut ring) = d.ring.lock() {
            for line in String::from_utf8_lossy(buf).lines() {
                if ring.len() >= RING_LINES {
                    ring.pop_front();
                }
                ring.push_back(line.to_string());
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The last `n` log lines of this run.
pub fn recent_log(n: usize) -> String {
    let Some(d) = DIAG.get() else { return String::new() };
    let Ok(ring) = d.ring.lock() else { return String::new() };
    let skip = ring.len().saturating_sub(n);
    ring.iter().skip(skip).cloned().collect::<Vec<_>>().join("\n")
}

fn tail_of(path: &Path, n: usize) -> String {
    let text = fs::read(path).map(|b| String::from_utf8_lossy(&b).to_string()).unwrap_or_default();
    let lines: Vec<&str> = text.lines().collect();
    lines[lines.len().saturating_sub(n)..].join("\n")
}

fn write_report(kind: &str, title: &str, body: &str) {
    let Some(d) = DIAG.get() else { return };
    let now = outpost_core::now();
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_millis())
        .unwrap_or(0);
    let id = format!("{kind}-{millis}");
    let text = format!(
        "{title}\n\nOutpost {} ({}) on {} {}\nTime: {}\n{body}\n",
        env!("CARGO_PKG_VERSION"),
        d.app,
        std::env::consts::OS,
        std::env::consts::ARCH,
        rfc3339(now),
    );
    let dir = crash_dir(&d.data_dir);
    let _ = fs::create_dir_all(&dir);
    let _ = fs::write(dir.join(format!("{id}.txt")), text);
    prune(&dir);
}

fn prune(dir: &Path) {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|r| r.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|x| x == "txt")).collect())
        .unwrap_or_default();
    if files.len() <= CRASH_KEEP {
        return;
    }
    files.sort();
    let extra = files.len() - CRASH_KEEP;
    for old in files.into_iter().take(extra) {
        let _ = fs::remove_file(old);
    }
}

/// One saved crash report.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct CrashSummary {
    /// e.g. `crash-1761470000000`.
    pub id: String,
    /// `crash` (an error Outpost caught) or `unclean` (it didn't close normally).
    pub kind: String,
    /// Unix seconds.
    pub at: i64,
    /// The first line saying what happened.
    pub summary: String,
}

/// Saved crash reports, newest first.
pub fn list_crashes(data_dir: &Path) -> Vec<CrashSummary> {
    let mut out: Vec<CrashSummary> = fs::read_dir(crash_dir(data_dir))
        .map(|r| r.flatten().map(|e| e.path()).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|p| {
            let id = p.file_stem()?.to_str()?.to_string();
            if p.extension()? != "txt" || !valid_id(&id) {
                return None;
            }
            let (kind, ms) = id.split_once('-')?;
            let at = ms.parse::<i64>().ok()? / 1000;
            let kind = kind.to_string();
            let text = fs::read_to_string(&p).unwrap_or_default();
            let summary = text
                .lines()
                .find_map(|l| l.strip_prefix("What: "))
                .unwrap_or_else(|| text.lines().next().unwrap_or(""))
                .chars()
                .take(200)
                .collect();
            Some(CrashSummary { id, kind, at, summary })
        })
        .collect();
    out.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
    out
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 60 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

pub fn read_crash(data_dir: &Path, id: &str) -> Option<String> {
    if !valid_id(id) {
        return None;
    }
    fs::read_to_string(crash_dir(data_dir).join(format!("{id}.txt"))).ok()
}

pub fn delete_crash(data_dir: &Path, id: &str) -> bool {
    valid_id(id) && fs::remove_file(crash_dir(data_dir).join(format!("{id}.txt"))).is_ok()
}

/// `2026-10-26T09:15:00Z` from Unix seconds (UTC).
pub fn rfc3339(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_written_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_761_470_100), "2025-10-26T09:15:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn crash_ids_are_checked() {
        assert!(valid_id("crash-1761470000000"));
        assert!(!valid_id("../outpost.db"));
        assert!(!valid_id("a/b"));
        assert!(!valid_id(""));
    }

    #[test]
    fn reports_are_listed_newest_first() {
        let dir = std::env::temp_dir().join(format!("outpost-diag-{}", std::process::id()));
        let crashes = crash_dir(&dir);
        fs::create_dir_all(&crashes).unwrap();
        fs::write(crashes.join("crash-1000000.txt"), "Outpost hit an error\nWhat: first\n").unwrap();
        fs::write(crashes.join("unclean-2000000.txt"), "Outpost didn't close\nWhat: second\n").unwrap();
        fs::write(crashes.join("notes.md"), "ignored").unwrap();
        let list = list_crashes(&dir);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, "unclean-2000000");
        assert_eq!(list[0].kind, "unclean");
        assert_eq!(list[0].at, 2000);
        assert_eq!(list[1].summary, "first");
        assert!(read_crash(&dir, "crash-1000000").unwrap().contains("first"));
        assert!(read_crash(&dir, "../../etc").is_none());
        assert!(delete_crash(&dir, "crash-1000000"));
        assert_eq!(list_crashes(&dir).len(), 1);
        let _ = fs::remove_dir_all(&dir);
    }
}

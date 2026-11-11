//! Debug record: one folder per run, one file per program, one line per event.
//!
//! ```text
//! <HH:MM:SS.mmm> [<level>] [<part>] <event> · <field>=<value> … · why: <why>
//! ```
//!
//! A log is in English and is not translated. Writing never fails
//! the caller: a record that cannot be written must not stop the program.
use std::{
    collections::BTreeSet,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant, SystemTime},
};

use chrono::{Local, NaiveDate};

use crate::paths::Data;

pub const ENV_RUN: &str = "WHAYBENCH_RUN";
pub const ENV_TRACE: &str = "WHAYBENCH_TRACE";
pub const MAX_BYTES: u64 = 5 * 1024 * 1024;
pub const MAX_LINE: usize = 2000;
pub const KEEP_RUNS: usize = 10;
pub const KEEP_SESSION_DAYS: u64 = 14;

pub const ERROR: &str = "error";
pub const WARN: &str = "warn";
pub const DECIDE: &str = "decide";
pub const DETAIL: &str = "detail";

struct Log {
    path: PathBuf,
    detail_file: PathBuf,
    day: Option<NaiveDate>,
    last: Option<chrono::DateTime<Local>>,
    over: bool,
    told: bool,
    detail: BTreeSet<String>,
    detail_read: Option<Instant>,
}

static LOG: Mutex<Option<Log>> = Mutex::new(None);

/// A new run's id, or the compositor's when this program was started by it.
pub fn run_id(data: &Data) -> String {
    if let Some(r) = std::env::var(ENV_RUN).ok().filter(|v| !v.is_empty()) { return r; }
    new_run_id(data)
}

/// A run id no folder has yet: the time now, with a number after it when that second is taken.
fn new_run_id(data: &Data) -> String {
    let base = Local::now().format("%Y%m%d-%H%M%S").to_string();
    let free = |n: &str| !data.log_runs().join(n).exists();
    if free(&base) { return base; }
    (2..).map(|i| format!("{base}-{i}")).find(|n| free(n)).unwrap()
}

/// Opens this program's file in the run's folder, tidies old records and writes the first header
/// line at once, before any other preparation can go wrong.
pub fn open(data: &Data, program: &str, run: &str, reason: &str) -> PathBuf {
    let folder = data.log_runs().join(run);
    let _ = fs::create_dir_all(&folder);
    let path = folder.join(format!("{program}.log"));
    *LOG.lock().unwrap() = Some(Log {
        path: path.clone(), detail_file: data.log_detail(), day: None, last: None,
        over: false, told: false, detail: BTreeSet::new(), detail_read: None,
    });
    header(&format!("Whaybench log · started={} reason={} run={} program={}", Local::now().format("%Y-%m-%d %H:%M:%S"), reason, run, program));
    let runs = prune_runs(&data.log_runs(), KEEP_RUNS, run).unwrap_or(0);
    let sessions = prune_sessions(&data.log_sessions(), Duration::from_secs(KEEP_SESSION_DAYS * 86400)).unwrap_or(0);
    if runs + sessions > 0 {
        decide("log", "old records removed", "over the keeping limit", &[("runs", runs.to_string()), ("session_files", sessions.to_string())]);
    }
    path
}

/// The clock as a record line starts with it.
pub fn clock() -> String { Local::now().format("%H:%M:%S%.3f").to_string() }

/// One header line (`# ...`).
pub fn header(text: &str) {
    if let Some(l) = LOG.lock().unwrap().as_mut() {
        write(l, &format!("# {}\n", text.replace('\n', " ")));
    }
}

fn write(l: &mut Log, text: &str) {
    let done = OpenOptions::new().create(true).append(true).open(&l.path).and_then(|mut f| f.write_all(text.as_bytes()));
    if let Err(e) = done {
        if !l.told {
            l.told = true;
            eprintln!("whaybench: cannot write the debug log ({}): {e}", l.path.display());
        }
    }
}

/// One field value: session ids cut to 8, spaces quoted, newlines flattened.
pub fn value(v: &str) -> String {
    if v.is_empty() { return "-".into(); }
    let v = v.replace('\n', " ");
    let is_id = v.len() == 36 && v.bytes().enumerate().all(|(i, b)| if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() && !b.is_ascii_uppercase() });
    if is_id { return v[..8].to_string(); }
    if v.contains(' ') { format!("'{v}'") } else { v }
}

/// A person's text or an answer: the first 80 characters and the full length.
pub fn clip(text: &str) -> (String, usize) {
    let flat = text.replace('\n', " ");
    let n = flat.chars().count();
    if n > 80 { (format!("{}…", flat.chars().take(80).collect::<String>()), n) } else { (flat, n) }
}

/// The line as it is written, without the clock.
pub fn format_body(level: &str, part: &str, event: &str, why: Option<&str>, fields: &[(&str, String)]) -> String {
    let mut t = format!("[{level}] [{part}] {event}");
    if !fields.is_empty() {
        let f: Vec<String> = fields.iter().map(|(k, v)| format!("{k}={}", value(v))).collect();
        t += &format!(" · {}", f.join(" "));
    }
    if let Some(w) = why { t += &format!(" · why: {}", w.replace('\n', " ")); }
    t
}

fn cut_line(text: String) -> String {
    if text.chars().count() > MAX_LINE { format!("{}…(cut)", text.chars().take(MAX_LINE).collect::<String>()) } else { text }
}

fn wanted(l: &mut Log, part: &str) -> bool {
    if l.detail_read.is_none_or(|t| t.elapsed() > Duration::from_secs(2)) {
        l.detail_read = Some(Instant::now());
        let chosen = fs::read_to_string(&l.detail_file).ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|v| v.get("parts").and_then(|p| p.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<BTreeSet<_>>()))
            .unwrap_or_default();
        let env = std::env::var(ENV_TRACE).unwrap_or_default();
        l.detail = chosen.into_iter().chain(env.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty())).collect();
    }
    l.detail.contains(part)
}

pub fn line(level: &str, part: &str, event: &str, why: Option<&str>, fields: &[(&str, String)]) {
    let mut guard = LOG.lock().unwrap();
    let Some(l) = guard.as_mut() else { return };
    if level == DETAIL && (l.over || !wanted(l, part)) { return; }
    let now = Local::now();
    let clock = now.format("%H:%M:%S%.3f");
    let mut out = String::new();
    if l.last.is_some_and(|t| now < t - chrono::Duration::seconds(1)) {
        out += &format!("{clock} [{WARN}] [log] clock went back\n");
    }
    l.last = Some(now);
    if l.day.is_some_and(|d| d != now.date_naive()) {
        out += &format!("# date {}\n", now.format("%Y-%m-%d"));
    }
    l.day = Some(now.date_naive());
    out += &cut_line(format!("{clock} {}", format_body(level, part, event, why, fields)));
    out.push('\n');
    write(l, &out);
    if !l.over && fs::metadata(&l.path).is_ok_and(|m| m.len() > MAX_BYTES) {
        l.over = true;
        write(l, &format!("{clock} [{WARN}] [log] size limit · size={MAX_BYTES} · why: no more detail lines in this run\n"));
    }
}

pub fn decide(part: &str, event: &str, why: &str, fields: &[(&str, String)]) { line(DECIDE, part, event, Some(why), fields); }
pub fn warn(part: &str, event: &str, why: &str, fields: &[(&str, String)]) { line(WARN, part, event, Some(why), fields); }
pub fn error(part: &str, event: &str, fields: &[(&str, String)]) { line(ERROR, part, event, None, fields); }
pub fn detail(part: &str, event: &str, fields: &[(&str, String)]) { line(DETAIL, part, event, None, fields); }

/// A crash leaves an error line with its place and the trace indented below, then behaves as usual.
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let message = info.payload().downcast_ref::<&str>().map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_default();
        error("run", "crashed", &[("place", place), ("error", message.chars().take(300).collect())]);
        let trace = std::backtrace::Backtrace::force_capture().to_string();
        if let Ok(mut g) = LOG.try_lock() {
            if let Some(l) = g.as_mut() {
                let t: String = trace.lines().map(|x| format!("    {x}\n")).collect();
                write(l, &t);
            }
        }
        previous(info);
    }));
}

/// Keeps the newest `keep` run folders (names sort by time). The current run always stays.
pub fn prune_runs(runs: &Path, keep: usize, current: &str) -> std::io::Result<usize> {
    let mut names: Vec<String> = fs::read_dir(runs)?.flatten().filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().to_string()).collect();
    names.sort();
    let mut removed = 0;
    let surplus = names.len().saturating_sub(keep);
    for n in names.into_iter().take(surplus) {
        if n != current && fs::remove_dir_all(runs.join(&n)).is_ok() { removed += 1; }
    }
    Ok(removed)
}

/// Session files not written for this long go.
pub fn prune_sessions(sessions: &Path, older_than: Duration) -> std::io::Result<usize> {
    let mut removed = 0;
    for e in fs::read_dir(sessions)?.flatten() {
        let old = e.metadata().and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok()).is_some_and(|age| age > older_than);
        if old && fs::remove_file(e.path()).is_ok() { removed += 1; }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("wb-log-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }
    #[test]
    fn values_follow_the_format() {
        assert_eq!(value("0a1b2c3d-0000-4000-8000-000000000001"), "0a1b2c3d");
        assert_eq!(value("a b"), "'a b'");
        assert_eq!(value("a\nb"), "'a b'");
        assert_eq!(value(""), "-");
        assert_eq!(value("abc"), "abc");
    }
    #[test]
    fn line_format_is_kept() {
        let b = format_body(DECIDE, "run", "start", Some("why"), &[("session", "0a1b2c3d-0000-4000-8000-000000000001".into()), ("window", "a b".into())]);
        assert_eq!(b, "[decide] [run] start · session=0a1b2c3d window='a b' · why: why");
        assert_eq!(format_body(ERROR, "run", "crashed", None, &[]), "[error] [run] crashed");
    }
    #[test]
    fn long_text_is_clipped() {
        let (t, n) = clip(&"é".repeat(100));
        assert_eq!(n, 100);
        assert_eq!(t.chars().count(), 81);
        assert!(cut_line("é".repeat(MAX_LINE + 5)).ends_with("…(cut)"));
        assert_eq!(cut_line("short".into()), "short");
    }
    #[test]
    fn only_the_newest_runs_stay_and_the_current_one_is_safe() {
        let d = tmp("runs");
        for i in 0..14 { fs::create_dir(d.join(format!("2026110{}-0000{:02}", 1, i))).unwrap(); }
        assert_eq!(prune_runs(&d, 10, "x").unwrap(), 4);
        assert_eq!(fs::read_dir(&d).unwrap().count(), 10);
        assert!(d.join("20261101-000013").exists());
        assert!(!d.join("20261101-000000").exists());
        let keep = tmp("runs2");
        for n in ["a", "b", "c"] { fs::create_dir(keep.join(n)).unwrap(); }
        prune_runs(&keep, 1, "a").unwrap();
        assert!(keep.join("a").exists() && keep.join("c").exists() && !keep.join("b").exists());
    }
    #[test]
    fn runs_in_the_same_second_get_their_own_folder() {
        let d = tmp("same");
        let data = Data::new(d.clone());
        // Not `run_id`: inside a Whaybench tab it is the id of the Whaybench that is running.
        let a = new_run_id(&data);
        fs::create_dir_all(data.log_runs().join(&a)).unwrap();
        let b = new_run_id(&data);
        assert_ne!(a, b);
    }
    #[test]
    fn missing_folders_do_not_fail_opening() {
        let d = tmp("open");
        let data = Data::new(d.join("deep/down"));
        let p = open(&data, "whaybench", "20261106-000000", "first");
        line(DECIDE, "run", "start", Some("test"), &[]);
        line(DETAIL, "run", "hidden", None, &[]);
        let text = fs::read_to_string(p).unwrap();
        assert!(text.starts_with("# Whaybench log · started="));
        assert!(text.contains("[decide] [run] start · why: test"));
        assert!(!text.contains("hidden"));
    }
}

//! Updating from inside: the latest release is looked up on GitHub at start and every six hours, and Whaybench → About
//! offers it when it is newer. Only for an install made by install.sh; a build from source updates itself with git.
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{Value, json};

use crate::{log as record, paths::Data};

pub const REPO: &str = "https://github.com/Whaybench/whaybench";
const EVERY: Duration = Duration::from_secs(6 * 3600);

fn latest_file(d: &Data) -> PathBuf { d.cache().join("latest-release.json") }

fn now_ms() -> u64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|t| t.as_millis() as u64).unwrap_or(0) }

/// Whether this program came from install.sh, which puts every program in ~/.local/lib/whaybench.
pub fn installed_release() -> bool {
    let (Ok(exe), Some(home)) = (std::env::current_exe(), std::env::var_os("HOME")) else { return false };
    exe.parent() == Some(Path::new(&home).join(".local/lib/whaybench").as_path())
}

/// The version in a release page's address: `…/releases/tag/v0.4.1` gives `0.4.1`.
fn tag_of(url: &str) -> Option<String> {
    let v = url.trim().rsplit_once("/tag/v")?.1;
    (!v.is_empty() && v.split('.').count() == 3 && v.split('.').all(|p| p.parse::<u64>().is_ok())).then(|| v.to_string())
}

/// Whether version `a` is newer than `b`, both as `main.minor.release`. A pre-release (with `-`) is never offered.
fn is_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Option<Vec<u64>> { v.split('.').map(|p| p.parse().ok()).collect() };
    match (parts(a), parts(b)) { (Some(x), Some(y)) => x > y, _ => false }
}

/// Asks GitHub which release is the latest: always at start (`at_start`), otherwise not again within six hours. Blocks
/// up to 15 seconds: run it off the screen's thread.
pub fn check(d: &Data, at_start: bool) {
    let last = std::fs::read_to_string(latest_file(d)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let recent = last.as_ref().and_then(|v| v["checked"].as_u64()).is_some_and(|at| now_ms().saturating_sub(at) < EVERY.as_millis() as u64);
    if recent && !at_start { return; }
    // The latest release's page redirects to its tag; only the address is read.
    let out = Command::new("curl").args(["-fsSL", "--max-time", "15", "-o", "/dev/null", "-w", "%{url_effective}", &format!("{REPO}/releases/latest")])
        .stdin(Stdio::null()).stderr(Stdio::null()).output();
    let version = out.ok().filter(|o| o.status.success()).and_then(|o| tag_of(&String::from_utf8_lossy(&o.stdout)));
    record::decide("update", "latest release", "the time to look again", &[("version", version.clone().unwrap_or_else(|| "unknown".into()))]);
    let kept = version.or_else(|| last.and_then(|v| v["version"].as_str().map(String::from)));
    let _ = crate::paths::write_atomic(&latest_file(d), json!({"version": kept, "checked": now_ms()}).to_string());
}

/// The latest release when it is newer than this program and this is an installed release.
pub fn newer(d: &Data) -> Option<String> {
    if !installed_release() { return None; }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(latest_file(d)).ok()?).ok()?;
    v["version"].as_str().filter(|v| is_newer(v, env!("CARGO_PKG_VERSION"))).map(String::from)
}

/// Installs the latest release with its install.sh, as a person would; its output goes to the logs. Blocks.
pub fn run(d: &Data) -> Result<(), String> {
    let log = d.logs().join("update.log");
    let _ = std::fs::create_dir_all(d.logs());
    let file = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let err = file.try_clone().map_err(|e| e.to_string())?;
    let status = Command::new("sh").arg("-c").arg(format!("curl -fsSL {REPO}/releases/latest/download/install.sh | sh"))
        .stdin(Stdio::null()).stdout(file).stderr(err).status().map_err(|e| e.to_string())?;
    record::decide("update", "installed", "the person chose the update", &[("ok", status.success().to_string())]);
    if status.success() { Ok(()) } else { Err(format!("{}", log.display())) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_tag_is_read_from_the_address_and_compared_by_number() {
        assert_eq!(tag_of("https://github.com/Whaybench/whaybench/releases/tag/v0.4.1\n").as_deref(), Some("0.4.1"));
        assert_eq!(tag_of("https://github.com/Whaybench/whaybench/releases/tag/v0.5.0-rc1"), None, "a pre-release is not offered");
        assert_eq!(tag_of("https://github.com/Whaybench/whaybench/releases"), None);
        assert!(is_newer("0.10.0", "0.9.3"), "by number, not by letters");
        assert!(is_newer("1.0.0", "0.4.0"));
        assert!(!is_newer("0.4.0", "0.4.0"));
        assert!(!is_newer("0.3.9", "0.4.0"));
    }
}

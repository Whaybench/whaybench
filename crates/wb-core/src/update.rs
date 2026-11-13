//! Updating from inside: the latest release is looked up on GitHub at start and every six hours. A newer one is
//! downloaded and made ready by itself, and installed at the next start, before the window opens: installing it while
//! Whaybench runs would give the running conversations' hooks the new program beside the old screen. Whaybench → About
//! offers it too. `"auto_update": false` in settings.json leaves it to the person. Only for an install made by
//! install.sh; a build from source updates itself with git.
use std::{path::{Path, PathBuf}, process::{Command, Stdio}, time::{Duration, SystemTime, UNIX_EPOCH}};

use serde_json::{Value, json};

use crate::{log as record, paths::Data};

pub const REPO: &str = "https://github.com/Whaybench/whaybench";
const EVERY: Duration = Duration::from_secs(6 * 3600);

fn latest_file(d: &Data) -> PathBuf { d.cache().join("latest-release.json") }
fn ready_dir(d: &Data) -> PathBuf { d.cache().join("update") }
const PACKAGE: &str = "whaybench-x86_64-linux.tar.gz";

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

/// The update the person chose (About → Update): the newer release is fetched and checked like an automatic one; the
/// restart that follows installs it. Never a script piped from the network. Blocks.
pub fn run(d: &Data) -> Result<(), String> {
    let v = newer(d).ok_or("no newer release")?;
    record::decide("update", "chosen", "the person chose the update", &[("version", v.clone())]);
    fetch(d, &v)
}

/// The key the releases are signed with (minisign). Its secret half is only in the release workflow.
const RELEASE_KEY: &str = "RWTwDOf9rBKzxEepAVlbLTan2g/RQFDuCZeqmU4Q+dNwf+wF+3a2Sfia";
/// What runs or is installed from a release: each comes with `<file>.minisig`.
const SIGNED: [&str; 2] = ["install.sh", PACKAGE];

/// A file of release `v` is the one the release workflow signed: the signature checks against the built-in key, and
/// its trusted comment names this file of this version, so a signed file of another release cannot stand in for it.
fn signed(key: &str, data: &[u8], signature: &str, file: &str, v: &str) -> Result<(), String> {
    let key = minisign_verify::PublicKey::from_base64(key).map_err(|e| format!("release key: {e}"))?;
    let sig = minisign_verify::Signature::decode(signature).map_err(|e| format!("{file}: signature: {e}"))?;
    key.verify(data, &sig, false).map_err(|e| format!("{file} of {v} is not signed by Whaybench: {e}"))?;
    let want = format!("whaybench {v} {file}");
    if sig.trusted_comment() != want { return Err(format!("{file}: signed for '{}', not '{want}'", sig.trusted_comment())); }
    Ok(())
}

/// install.sh as the person would run it: with their umask, not Whaybench's private one.
fn install_sh(script: &str) -> Command {
    let mut c = Command::new("sh");
    let umask = crate::paths::user_umask();
    // SAFETY: umask is async-signal-safe.
    unsafe { std::os::unix::process::CommandExt::pre_exec(&mut c, move || { libc::umask(umask); Ok(()) }); }
    c.arg("-c").arg(script).stdin(Stdio::null());
    c
}

/// Whether a newer release is fetched by itself: unless settings.json says `"auto_update": false`.
pub fn automatic(d: &Data) -> bool {
    std::fs::read_to_string(d.root.join("settings.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_none_or(|v| v["auto_update"] != false)
}

/// The release made ready for the next start, when it is newer than this program.
pub fn ready(d: &Data) -> Option<String> {
    if !installed_release() { return None; }
    std::fs::read_to_string(ready_dir(d).join("version")).ok().map(|v| v.trim().to_string()).filter(|v| is_newer(v, env!("CARGO_PKG_VERSION")))
}

/// Downloads release `v` (its install.sh and package) and checks both against their signatures, for the next start.
/// Blocks for the download: run it off the screen's thread.
pub fn fetch(d: &Data, v: &str) -> Result<(), String> {
    if ready(d).as_deref() == Some(v) { return Ok(()); }
    let part = d.cache().join("update.part");
    let _ = std::fs::remove_dir_all(&part);
    std::fs::create_dir_all(&part).map_err(|e| e.to_string())?;
    for file in ["install.sh", PACKAGE, &format!("{PACKAGE}.sha256"), "install.sh.minisig", &format!("{PACKAGE}.minisig")] {
        let ok = Command::new("curl").args(["-fsSL", "--max-time", "600", "-o", file, &format!("{REPO}/releases/download/v{v}/{file}")])
            .current_dir(&part).stdin(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
        if !ok { return Err(format!("{file} of {v} did not download")); }
    }
    for file in SIGNED {
        let read = std::fs::read(part.join(file)).and_then(|b| std::fs::read_to_string(part.join(format!("{file}.minisig"))).map(|s| (b, s)));
        let ok = read.map_err(|e| e.to_string()).and_then(|(b, s)| signed(RELEASE_KEY, &b, &s, file, v));
        if let Err(e) = ok { let _ = std::fs::remove_dir_all(&part); record::warn("update", "not signed", &e, &[("version", v.to_string())]); return Err(e); }
    }
    let checked = Command::new("sha256sum").args(["-c", "--quiet", &format!("{PACKAGE}.sha256")]).current_dir(&part)
        .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    if !checked { let _ = std::fs::remove_dir_all(&part); return Err(format!("the package of {v} does not match its checksum")); }
    std::fs::write(part.join("version"), v).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_dir_all(ready_dir(d));
    std::fs::rename(&part, ready_dir(d)).map_err(|e| e.to_string())?;
    record::decide("update", "ready", "a newer release was found; it is installed at the next start", &[("version", v.to_string())]);
    Ok(())
}

/// At start, before the window: installs the release made ready, with its own install.sh (output in update.log).
/// True when it was installed and the program should start again as the new one. A ready release no longer newer
/// (installed meanwhile from Whaybench → About) is removed.
pub fn install_ready(d: &Data) -> bool {
    let dir = ready_dir(d);
    if !dir.exists() { return false; }
    let Some(v) = ready(d) else { let _ = std::fs::remove_dir_all(&dir); return false };
    let _ = std::fs::create_dir_all(d.logs());
    let Ok(log) = std::fs::File::create(d.logs().join("update.log")) else { return false };
    let Ok(err) = log.try_clone() else { return false };
    let ok = install_sh("sh ./install.sh").current_dir(&dir).stdout(log).stderr(err).status().is_ok_and(|s| s.success());
    // Once only: a failed one would hold up every start; the next check fetches it again.
    let _ = std::fs::remove_dir_all(&dir);
    if ok { record::decide("update", "installed", "a release was ready at start", &[("version", v)]); }
    else { record::warn("update", "not installed", "install.sh failed at start; see update.log", &[("version", v)]); }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_update_runs_only_what_the_release_key_signed_for_that_version_and_file() {
        // A key made for this test only, and install.sh signed with it for 9.9.9 and, separately, for 9.9.8.
        const KEY: &str = "RWQ9Cg5ooQtD22yrR/PfMb/N7uQfxTZ+qB8PHxTRSye4ykRxmdEgeJ7B";
        const FOR_999: &str = "untrusted comment: signature from minisign secret key\nRUQ9Cg5ooQtD24+rDUaIMXqMtcBhvUghr98znoqwPG6b5fjUq5Bq4Ta1cHtiPW2W3xmis7HWPB1UKYqQJ5QPoT9ixt/eNhhibwc=\ntrusted comment: whaybench 9.9.9 install.sh\nFTyB5tvGeSP5SAfWHPFYRoz9S0YLVFEIR4FihVWbHnZ1dSrFqWPqEFv85LU1F2a6ppM2dI/DiD8L3r4W8FMnBg==\n";
        const FOR_998: &str = "untrusted comment: signature from minisign secret key\nRUQ9Cg5ooQtD24+rDUaIMXqMtcBhvUghr98znoqwPG6b5fjUq5Bq4Ta1cHtiPW2W3xmis7HWPB1UKYqQJ5QPoT9ixt/eNhhibwc=\ntrusted comment: whaybench 9.9.8 install.sh\n+M2HAfcscj35mVsMC/XRfBKrXbjzXOLVC2694QtSWYAvO9LH1QiA4Z29TFKh0oxCaqpnZHwIIUpdG0vhGn3aDQ==\n";
        let file = b"echo install";
        assert!(signed(KEY, file, FOR_999, "install.sh", "9.9.9").is_ok());
        assert!(signed(KEY, b"echo changed", FOR_999, "install.sh", "9.9.9").is_err(), "a changed file");
        assert!(signed(KEY, file, FOR_998, "install.sh", "9.9.9").is_err(), "a signed file of another release");
        assert!(signed(KEY, file, FOR_999, PACKAGE, "9.9.9").is_err(), "a signature for another file");
        assert!(signed(RELEASE_KEY, file, FOR_999, "install.sh", "9.9.9").is_err(), "another key");
        assert!(minisign_verify::PublicKey::from_base64(RELEASE_KEY).is_ok(), "the release key is a minisign key");
    }

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

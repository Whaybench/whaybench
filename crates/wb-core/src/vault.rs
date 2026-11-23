//! A Briefsafe project's vault (spec 3.22): its data encrypted with gocryptfs in `<folder>/.whaybench`,
//! opened by the host in `$XDG_RUNTIME_DIR/whaybench/<container name>/<id>` while the project is open. The key is a
//! random one kept in the keyring; it goes to gocryptfs on standard input only, never on a command line or in a file.
//! The recovery key (gocryptfs's master key) is shown once, when the vault is made.
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use crate::roots::Stub;

/// The keyring entry of a project's vault key: one per project of one data folder.
pub fn key_name(s: &Stub) -> String { format!("vault-{}-{}", s.id, s.made) }

/// The folder bound into the project's container over its `cache/open` (`rslave`: the vault mounted in it shows
/// there), private to the person.
pub fn run_dir(s: &Stub) -> PathBuf {
    // The person's runtime folder by their user number, not by a variable each program may have or not (it would
    // put the vault somewhere else, or in /tmp where another user could be first): the same for every program.
    // Safety: getuid has no preconditions.
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/run/user/{uid}/whaybench")).join(format!("{}-{}", s.id, s.made))
}

/// Where the vault is open: the project's data root then.
pub fn mountpoint(s: &Stub) -> PathBuf { run_dir(s).join(&s.id) }

/// Where the encrypted data rests.
pub fn cipher_dir(s: &Stub) -> PathBuf { s.folder.join(crate::roots::PROJECT_DATA) }

fn run(args: &[&str], input: &str) -> Result<String, String> {
    let mut child = Command::new(args[0]).args(&args[1..]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped())
        .spawn().map_err(|e| format!("{}: {e}", args[0]))?;
    // The key on standard input only.
    child.stdin.take().ok_or("no input")?.write_all(input.as_bytes()).map_err(|e| e.to_string())?;
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()) } else { Err(format!("{}: {}", args[0], String::from_utf8_lossy(&out.stderr).trim())) }
}

fn new_key() -> Result<String, String> {
    use std::io::Read;
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).map_err(|e| e.to_string())?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn private_dir(p: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(p).map_err(|e| format!("{}: {e}", p.display()))?;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}

/// Makes the vault in the empty data folder, keeps its key in the keyring, and answers the recovery key.
pub fn create(s: &Stub) -> Result<String, String> {
    let cipher = cipher_dir(s);
    private_dir(&cipher)?;
    if std::fs::read_dir(&cipher).map_err(|e| e.to_string())?.next().is_some() { return Err(format!("{} is not empty", cipher.display())); }
    let key = new_key()?;
    let c = cipher.to_string_lossy().to_string();
    run(&["gocryptfs", "-init", "-q", &c], &format!("{key}\n"))?;
    let recovery = run(&["gocryptfs-xray", "-dumpmasterkey", &format!("{c}/gocryptfs.conf")], &format!("{key}\n"))?;
    crate::secret::set(&key_name(s), &format!("Whaybench project {}", s.name), &key)?;
    Ok(recovery)
}

/// Shows the recovery key to the person once, never to whoever ran the command (an AI's transcript keeps what it
/// reads): on a page in the view panel, from a file in the person's runtime folder (memory, private) that goes once
/// the panel has it; without a panel on the terminal itself; without either the file stays there, said by path.
/// Whether a path is a recovery key page `show_recovery` wrote (in the person's runtime folder, nowhere else): the
/// view shows it whatever it follows, keeps it out of its lists and never reads it out to a tool.
pub fn is_recovery_page(path: &str) -> bool {
    let p = Path::new(path.strip_prefix("file://").unwrap_or(path));
    // Safety: getuid has no preconditions.
    let base = PathBuf::from(format!("/run/user/{}/whaybench", unsafe { libc::getuid() }));
    p.parent() == Some(base.as_path()) && p.file_name().is_some_and(|n| { let n = n.to_string_lossy(); n.starts_with("recovery-") && n.ends_with(".md") })
}

/// Answers what the command may print.
pub fn show_recovery(s: &Stub, host: &crate::paths::Data, recovery: &str) -> String {
    use std::os::unix::fs::OpenOptionsExt;
    let t = |k: &str| crate::i18n::f(k, &[("name", s.name.clone())]);
    let page = format!("# {}\n\n{}\n\n```\n{recovery}\n```\n\n{}\n", t("briefsafe-recovery-title"), t("briefsafe-recovery-intro"), t("briefsafe-recovery-keep"));
    let Some(base) = run_dir(s).parent().map(Path::to_path_buf) else { return t("briefsafe-recovery-not-shown") };
    let file = base.join(format!("recovery-{}-{}.md", s.id, s.made));
    let written = private_dir(&base).ok().and_then(|_| std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&file).ok())
        .is_some_and(|mut f| f.write_all(page.as_bytes()).is_ok());
    if written && crate::view::show_and_wait(host, &file.to_string_lossy(), std::time::Duration::from_secs(10)) {
        // The panel reads it as it shows it.
        std::thread::sleep(std::time::Duration::from_secs(3));
        let _ = std::fs::remove_file(&file);
        return t("briefsafe-recovery-in-view");
    }
    if let Ok(mut tty) = std::fs::OpenOptions::new().write(true).open("/dev/tty") && tty.write_all(page.as_bytes()).is_ok() {
        let _ = std::fs::remove_file(&file);
        return t("briefsafe-recovery-on-terminal");
    }
    if written { crate::i18n::f("briefsafe-recovery-in-file", &[("path", file.display().to_string())]) } else { t("briefsafe-recovery-not-shown") }
}

/// Whether the vault is open now (mounted where it opens).
pub fn mounted(s: &Stub) -> bool {
    let at = mountpoint(s).to_string_lossy().to_string();
    // The mount point as /proc/self/mountinfo writes it (octal escapes for space and the like).
    let escaped = at.replace('\\', "\\134").replace(' ', "\\040").replace('\t', "\\011").replace('\n', "\\012");
    std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|t| t.lines().any(|l| l.split(' ').nth(4) == Some(escaped.as_str())))
}

/// What a vault cannot do without and is not here, as a message for the person; None when one can be made: a container
/// for its agents (its key never reaches a host-run one), gocryptfs with FUSE to open it, a keyring for its key (a
/// locked one is there).
pub fn missing() -> Option<String> {
    let runs = |program: &str, arg: &str| Command::new(program).arg(arg).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success());
    let need = |k: &str| Some(crate::i18n::t(k).to_string());
    if !crate::container::podman_found() { return need("vault-needs-podman"); }
    if crate::container::host_image().is_none() { return need("vault-needs-image"); }
    if !runs("gocryptfs", "-version") { return need("vault-needs-gocryptfs"); }
    if !runs("fusermount3", "-V") { return need("vault-needs-fuse"); }
    match crate::secret::state() { crate::secret::State::Absent(e) => Some(crate::i18n::f("vault-needs-keyring", &[("why", e)])), _ => None }
}

/// Whether a vault has been made with this data folder: then one that cannot be made now is a fault to be told, not a
/// computer without vaults.
pub fn made_here(host: &crate::paths::Data) -> bool { host.root.join("vault-made").exists() || crate::roots::stubs(host).iter().any(|s| s.vault) }
pub fn note_made(host: &crate::paths::Data) { let _ = std::fs::write(host.root.join("vault-made"), ""); }

/// Open and answering: mounted, and its gocryptfs still there (a dead one leaves the mount, which then fails).
pub fn ready(s: &Stub) -> bool { mounted(s) && std::fs::metadata(mountpoint(s)).is_ok() }

/// Opens the vault with the key from the keyring. None in the keyring: `NO_KEY`.
pub const NO_KEY: &str = "no key for this project in the keyring";
pub fn mount(s: &Stub) -> Result<(), String> {
    if ready(s) { return Ok(()); }
    // A mount whose gocryptfs died ("transport endpoint is not connected"): taken off before opening again.
    if mounted(s) { let _ = Command::new("fusermount3").args(["-u", "-z", &mountpoint(s).to_string_lossy()]).output(); }
    let key = crate::secret::get(&key_name(s))?.ok_or(NO_KEY)?;
    mount_with(s, &[], &key)
}

fn mount_with(s: &Stub, flags: &[&str], secret: &str) -> Result<(), String> {
    private_dir(&run_dir(s))?;
    private_dir(&mountpoint(s))?;
    let (c, m) = (cipher_dir(s).to_string_lossy().to_string(), mountpoint(s).to_string_lossy().to_string());
    // Its messages to a file of the person's, not a pipe: gocryptfs going to the background may keep its output open
    // (when it cannot hand it to syslog), and a pipe would then never end.
    let log = run_dir(s).join("mount.log");
    let errs = std::fs::File::create(&log).map_err(|e| e.to_string())?;
    let mut child = Command::new("gocryptfs").arg("-q").args(flags).args([&c, &m]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(errs)
        .spawn().map_err(|e| format!("gocryptfs: {e}"))?;
    child.stdin.take().ok_or("no input")?.write_all(format!("{secret}\n").as_bytes()).map_err(|e| e.to_string())?;
    let status = child.wait().map_err(|e| e.to_string())?;
    let said = std::fs::read_to_string(&log).unwrap_or_default();
    let _ = std::fs::remove_file(&log);
    if status.success() { Ok(()) } else { Err(format!("gocryptfs: {}", said.trim())) }
}

/// Opens the vault with its recovery key, then gives it a new key, kept in the keyring (the old one is gone).
/// gocryptfs takes any master key here without a check: the new key is kept only once the vault opened with it shows
/// this project's own marker inside; otherwise the vault's key file is put back as it was and nothing is kept.
pub fn mount_with_recovery(s: &Stub, recovery: &str) -> Result<(), String> {
    // One line of hex (gocryptfs prints it with dashes too): a second line would be taken for the new password.
    let recovery: String = recovery.trim().chars().filter(|c| *c != '-').collect();
    if recovery.len() != 64 || !recovery.chars().all(|c| c.is_ascii_hexdigit()) { return Err("a recovery key is 64 hexadecimal digits on one line".into()); }
    if mounted(s) { return Err("the vault is open already".into()); }
    // Tried first, read-only, before the vault's key file is touched: a key that does not show this project's marker
    // changes nothing at all.
    let tried = mount_with(s, &["-masterkey=stdin", "-ro"], &recovery).map(|_| s.intact_open());
    let _ = unmount(s);
    if mounted(s) { return Err("the vault could not be closed after trying the recovery key".into()); }
    if !matches!(tried, Ok(true)) { return Err("that recovery key does not open this project's vault".into()); }
    let cipher = cipher_dir(s);
    let (conf, bak) = (cipher.join("gocryptfs.conf"), cipher.join("gocryptfs.conf.bak"));
    // One left by an earlier try that stopped half way: kept aside, never thrown away (it may be the one that opens).
    if bak.exists() { std::fs::rename(&bak, cipher.join(format!("gocryptfs.conf.bak-{}", crate::clock::now_ms()))).map_err(|e| e.to_string())?; }
    let key = new_key()?;
    run(&["gocryptfs", "-passwd", "-masterkey=stdin", &cipher.to_string_lossy()], &format!("{recovery}\n{key}\n"))?;
    let undo = |why: String| -> String {
        let _ = unmount(s);
        match std::fs::rename(&bak, &conf) { Ok(()) => why, Err(e) => format!("{why}; and the vault's key file could not be put back from {}: {e}", bak.display()) }
    };
    if let Err(e) = mount_with(s, &[], &key) { return Err(undo(e)); }
    if !s.intact_open() { return Err(undo("that recovery key does not open this project's vault".into())); }
    if let Err(e) = crate::secret::set(&key_name(s), &format!("Whaybench project {}", s.name), &key) { return Err(undo(format!("the new key could not be kept in the keyring: {e}"))); }
    // Kept: the old key file (opened by the old key) goes.
    let _ = std::fs::remove_file(&bak);
    Ok(())
}

/// Closes the vault. Something still working in it keeps it open, and says so.
pub fn unmount(s: &Stub) -> Result<(), String> {
    if !mounted(s) { return Ok(()); }
    let m = mountpoint(s).to_string_lossy().to_string();
    let out = Command::new("fusermount3").args(["-u", &m]).output().map_err(|e| format!("fusermount3: {e}"))?;
    if out.status.success() { Ok(()) } else { Err(format!("the project's vault stays open: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

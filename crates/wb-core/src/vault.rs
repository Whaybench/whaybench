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
pub fn key_name(s: &Stub) -> String { format!("vault-{}", crate::container::name(s)) }

/// The folder bound into the project's container over its `cache/open` (`rslave`: the vault mounted in it shows
/// there), private to the person.
pub fn run_dir(s: &Stub) -> PathBuf {
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()).map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    runtime.join("whaybench").join(crate::container::name(s))
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

/// Whether the vault is open now (mounted where it opens).
pub fn mounted(s: &Stub) -> bool {
    let at = mountpoint(s).to_string_lossy().to_string();
    // The mount point as /proc/self/mountinfo writes it (octal escapes for space and the like).
    let escaped = at.replace('\\', "\\134").replace(' ', "\\040").replace('\t', "\\011").replace('\n', "\\012");
    std::fs::read_to_string("/proc/self/mountinfo").is_ok_and(|t| t.lines().any(|l| l.split(' ').nth(4) == Some(escaped.as_str())))
}

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
    let args: Vec<&str> = ["gocryptfs", "-q"].into_iter().chain(flags.iter().copied()).chain([c.as_str(), m.as_str()]).collect();
    run(&args, &format!("{secret}\n")).map(|_| ())
}

/// Opens the vault with its recovery key, then gives it a new key, kept in the keyring (the old one is gone).
/// gocryptfs takes any master key here without a check: the new key is kept only once the vault opened with it shows
/// this project's own marker inside; otherwise the vault's key file is put back as it was and nothing is kept.
pub fn mount_with_recovery(s: &Stub, recovery: &str) -> Result<(), String> {
    // One line of hex (gocryptfs prints it with dashes too): a second line would be taken for the new password.
    let recovery: String = recovery.trim().chars().filter(|c| *c != '-').collect();
    if recovery.len() != 64 || !recovery.chars().all(|c| c.is_ascii_hexdigit()) { return Err("a recovery key is 64 hexadecimal digits on one line".into()); }
    if mounted(s) { return Err("the vault is open already".into()); }
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

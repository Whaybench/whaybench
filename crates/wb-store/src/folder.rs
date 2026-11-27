//! Project folders (spec 3.22): a project folder is a btrfs subvolume, its snapshots are read-only subvolumes in a
//! hidden folder beside it (`.<name>.snapshots/<number>`). Where the file system is not btrfs the folder is a plain
//! directory and has no snapshots.
use std::{
    ffi::OsStr,
    fs,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{Res, now_ms};

/// Snapshots kept per project; the oldest go first. Each is whole, not a step on another, so any can go.
const SNAPSHOT_LIMIT: usize = 30;

/// Old snapshots deleted at most in one save (file by file each can take minutes).
const PRUNE_AT_ONCE: usize = 3;

/// The inode of every btrfs subvolume's top directory.
const SUBVOLUME_INODE: u64 = 256;

fn output(program: &str, args: &[&OsStr]) -> Res<String> {
    let out = Command::new(program).args(args).output().map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()) } else { Err(format!("{program}: {}", String::from_utf8_lossy(&out.stderr).trim())) }
}

fn run(program: &str, args: &[&OsStr]) -> Res<()> { output(program, args).map(|_| ()) }

fn btrfs(args: &[&OsStr]) -> Res<()> { run("btrfs", args) }

fn on_btrfs(path: &Path) -> bool { output("stat", &["-f".as_ref(), "-c".as_ref(), "%T".as_ref(), path.as_os_str()]).is_ok_and(|t| t == "btrfs") }

/// Read-only, as a snapshot is: writing there is refused by the file system (EROFS).
fn read_only(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // Safety: a path as a C string, valid for the call.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) != 0 && *libc::__errno_location() == libc::EROFS }
}

pub fn is_subvolume(path: &Path) -> bool { fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.ino() == SUBVOLUME_INODE) }

/// Makes the project folder: a subvolume on btrfs, else a plain folder. With `from`, everything in that folder is
/// copied in (reflinked where it can be). Answers whether the folder is a subvolume.
pub fn make(dir: &Path, from: Option<&Path>) -> Res<bool> {
    if fs::symlink_metadata(dir).is_ok() { return Err(format!("folder exists: {}", dir.display())); }
    if let Some(src) = from {
        let src = fs::canonicalize(src).map_err(|e| format!("{}: {e}", src.display()))?;
        if !src.is_dir() { return Err(format!("not a folder: {}", src.display())); }
        let parent = dir.parent().map(|p| fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())).unwrap_or_default();
        if parent.starts_with(&src) { return Err(format!("the project folder cannot be inside the folder copied from: {}", src.display())); }
    }
    let parent = dir.parent().ok_or("no parent folder")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    // Without the btrfs tools a plain folder, as on another file system: no snapshots.
    let subvolume = on_btrfs(parent) && output("btrfs", &["--version".as_ref()]).is_ok();
    if subvolume { btrfs(&["subvolume".as_ref(), "create".as_ref(), dir.as_os_str()])?; } else { fs::create_dir(dir).map_err(|e| e.to_string())?; }
    if let Some(src) = from
        && let Err(e) = copy_into(src, dir) {
        // The copy failed half way: what was made goes, the source stays as it was.
        return Err(match discard(dir, subvolume) { Ok(()) => e, Err(e2) => format!("{e}; the half copy is left at {}: {e2}", dir.display()) });
    }
    Ok(subvolume)
}

/// Copies everything in `src` into `dst` (reflinked where it can be), keeping modes and times.
pub fn copy_into(src: &Path, dst: &Path) -> Res<()> {
    run("cp", &["-a".as_ref(), "--reflink=auto".as_ref(), src.join(".").as_os_str(), dst.as_os_str()])
}

/// Removes a project folder just made, when making the project failed after it.
pub fn discard(dir: &Path, subvolume: bool) -> Res<()> {
    // A user's own subvolume can also go by emptying it and removing its folder (Linux 4.18 on), without
    // user_subvol_rm_allowed.
    if subvolume && btrfs(&["subvolume".as_ref(), "delete".as_ref(), dir.as_os_str()]).is_ok() { return Ok(()); }
    wipe(dir)
}

/// Removes a folder file by file, its read-only folders made writable first; links are not followed.
fn wipe(dir: &Path) -> Res<()> {
    writable(dir);
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
}

/// Folders the owner cannot write, read or enter (Go's module cache, a test's mode 000) opened up, so what is in them
/// can be deleted; only those are touched. One by one (`;`), so each is opened before find goes into it.
fn writable(dir: &Path) {
    let _ = run("find", &[dir.as_os_str(), "-type".as_ref(), "d".as_ref(), "!".as_ref(), "-perm".as_ref(), "-u=rwx".as_ref(), "-exec".as_ref(), "chmod".as_ref(), "u+rwx".as_ref(), "{}".as_ref(), ";".as_ref()]);
}

/// Empties a project folder or a snapshot but for its marker: stopped part way, what is left still says whose it is.
fn empty_but_marker(dir: &Path) -> Res<()> {
    writable(dir);
    let data = dir.join(wb_core::roots::PROJECT_DATA);
    let gone = |p: &Path| if fs::symlink_metadata(p).is_ok_and(|m| m.is_dir()) { fs::remove_dir_all(p) } else { fs::remove_file(p) };
    for e in fs::read_dir(dir).map_err(|e| e.to_string())?.flatten() {
        if e.path() != data { gone(&e.path()).map_err(|x| format!("{}: {x}", e.path().display()))?; }
    }
    for e in fs::read_dir(&data).map_err(|e| e.to_string())?.flatten() {
        if e.file_name() != wb_core::roots::MARKER { gone(&e.path()).map_err(|x| format!("{}: {x}", e.path().display()))?; }
    }
    Ok(())
}

/// The project a folder (or a snapshot of it) belongs to: its data root's marker (the id and when it was made).
fn owner(dir: &Path) -> Option<serde_json::Value> {
    let bytes = wb_core::beneath::read(dir, &Path::new(wb_core::roots::PROJECT_DATA).join(wb_core::roots::MARKER), 4096).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v["id"].is_string().then(|| serde_json::json!({"id": v["id"], "made": v["made"]}))
}

/// Renames without replacing anything there (made again since it was checked).
fn rename_new(from: &Path, to: &Path) -> Res<()> {
    use std::os::unix::ffi::OsStrExt;
    let c = |p: &Path| std::ffi::CString::new(p.as_os_str().as_bytes()).map_err(|e| e.to_string());
    let (f, t) = (c(from)?, c(to)?);
    // Safety: two paths as C strings, valid for the call.
    if unsafe { libc::renameat2(libc::AT_FDCWD, f.as_ptr(), libc::AT_FDCWD, t.as_ptr(), libc::RENAME_NOREPLACE) } != 0 {
        return Err(format!("{}: {}", to.display(), std::io::Error::last_os_error()));
    }
    Ok(())
}

pub fn snapshot_dir(dir: &Path) -> PathBuf {
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dir.with_file_name(format!(".{name}.snapshots"))
}

/// When the newest snapshot was taken (its name is the time), by the names alone: cheap enough to show all the time.
pub fn last_snapshot(dir: &Path) -> Option<i64> {
    fs::read_dir(snapshot_dir(dir)).ok()?.flatten().filter_map(|e| e.file_name().to_str()?.parse().ok()).max()
}

/// The snapshots of a project folder, oldest first: subvolumes with a number for a name, never a link, of this
/// project (one left beside a folder of the same name by an earlier project is not this one's to restore or prune).
pub fn snapshots(dir: &Path) -> Vec<String> {
    // A folder of no project has no snapshots of its own to tell apart.
    owner(dir).map(|mine| numbered(dir, &mine, true)).unwrap_or_default()
}

/// Subvolumes with a number for a name beside a folder, of the project `mine` (its id and made): read-only ones are
/// its snapshots, writable ones snapshots whose delete stopped part way (made writable is where a delete begins).
fn numbered(dir: &Path, mine: &serde_json::Value, ro: bool) -> Vec<String> {
    let at = snapshot_dir(dir);
    if !fs::symlink_metadata(&at).is_ok_and(|m| m.is_dir()) { return Vec::new(); }
    let mut names: Vec<String> = fs::read_dir(&at).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.parse::<u64>().is_ok() && is_subvolume(&at.join(n)) && read_only(&at.join(n)) == ro && owner(&at.join(n)).as_ref() == Some(mine)).collect();
    names.sort_by_key(|n| n.parse::<u64>().unwrap_or(0));
    names
}

/// A snapshot taken, and what could not be cleaned up after it (the step itself worked).
pub struct Taken { pub name: String, pub warning: Option<String> }

/// Takes a read-only snapshot, then keeps no more than 30.
pub fn snapshot(dir: &Path) -> Res<Taken> {
    let name = take(dir)?;
    Ok(Taken { name, warning: prune_old(dir) })
}

/// Keeps no more than 30 snapshots, deletes those left half deleted: what could not be deleted, said. Apart from
/// `take`, so a project's container is paused only for the snapshot itself (deleting file by file can take minutes).
pub fn prune_old(dir: &Path) -> Option<String> { prune(dir, SNAPSHOT_LIMIT).err() }

/// Deletes this project's snapshots whose delete stopped part way.
fn left_over(dir: &Path) -> Res<()> {
    let Some(mine) = owner(dir) else { return Ok(()) };
    for name in numbered(dir, &mine, false) { delete_snapshot(&snapshot_dir(dir).join(&name)).map_err(|e| format!("snapshot {name} is left half deleted: {e}"))?; }
    drop_empty(dir);
    Ok(())
}

/// A delete stopped after the marker went leaves an empty writable subvolume no one can be told by: only an empty one
/// goes (a folder is removed only when empty), so nothing of anyone's is lost.
fn drop_empty(dir: &Path) {
    let at = snapshot_dir(dir);
    for e in fs::read_dir(&at).into_iter().flatten().flatten() {
        let p = e.path();
        if e.file_name().to_str().is_some_and(|n| n.parse::<u64>().is_ok()) && is_subvolume(&p) && !read_only(&p) { let _ = fs::remove_dir(&p); }
    }
}

pub fn take(dir: &Path) -> Res<String> {
    if !is_subvolume(dir) { return Err(format!("no snapshot: {} is not a btrfs subvolume", dir.display())); }
    // A snapshot holds an inner subvolume as an empty folder: it would not be the whole project.
    if let Some(inner) = nested_subvolume(dir)? { return Err(format!("no snapshot: a subvolume is inside the project folder ({inner}), and a snapshot would not hold it")); }
    let at = snapshot_dir(dir);
    if fs::symlink_metadata(&at).is_ok_and(|m| !m.is_dir()) { return Err(format!("{} is not a folder", at.display())); }
    fs::create_dir_all(&at).map_err(|e| e.to_string())?;
    // Always after the last one, so a clock set back cannot put a new snapshot first in line for pruning.
    let last = snapshots(dir).last().and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    let mut number = (now_ms().max(0) as u64).max(last + 1);
    while fs::symlink_metadata(at.join(number.to_string())).is_ok() { number += 1; }
    let name = number.to_string();
    btrfs(&["subvolume".as_ref(), "snapshot".as_ref(), "-r".as_ref(), dir.as_os_str(), at.join(&name).as_os_str()])?;
    Ok(name)
}

/// Deletes a snapshot. Made writable first: that takes it off the list and out of restores at once (only read-only
/// ones are snapshots), and it is never made read-only again, half emptied. Then at once where the file system is
/// mounted `user_subvol_rm_allowed`, else emptied like a folder, its marker last (Linux 4.18 on: no root, only more
/// time). Forced, as one received from another disk (`btrfs receive`) needs.
fn delete_snapshot(path: &Path) -> Res<()> {
    let ro_off = |force: &[&OsStr]| btrfs(&[&["property".as_ref(), "set".as_ref()], force, &["-ts".as_ref(), path.as_os_str(), "ro".as_ref(), "false".as_ref()]].concat());
    // btrfs-progs before 5.14.2 has no -f: then without it.
    ro_off(&["-f".as_ref()]).or_else(|_| ro_off(&[]))?;
    if btrfs(&["subvolume".as_ref(), "delete".as_ref(), path.as_os_str()]).is_ok() { return Ok(()); }
    empty_but_marker(path)?;
    fs::remove_dir_all(path).map_err(|e| e.to_string())
}

/// The folder's size as `du -sh` says it (snapshots apart), for the person to weigh before a delete. `du` fails on
/// one unreadable file yet still prints the total.
pub fn size(dir: &Path) -> String {
    Command::new("du").arg("-sh").arg(dir).output().ok()
        .and_then(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().next().map(String::from)).unwrap_or_default()
}

/// What is mounted inside a folder (a network share, a USB disk): emptying the folder file by file would go into it
/// and delete what is not the project's. From `/proc/self/mounts`, read as bytes (a path need not be UTF-8), whose
/// spaces and the like are octal (`\040`). Not read: an error, never taken for nothing mounted.
pub fn mounts_inside(dir: &Path) -> Res<Vec<String>> {
    use std::os::unix::ffi::OsStrExt;
    let dir = fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    let table = fs::read("/proc/self/mounts").map_err(|e| format!("cannot read what is mounted (/proc/self/mounts): {e}"))?;
    Ok(table.split(|&b| b == b'\n').filter_map(|l| l.split(|&b| b == b' ').nth(1)).map(|m| PathBuf::from(OsStr::from_bytes(&unescape_octal(m))))
        .filter(|m| m != &dir && m.starts_with(&dir)).map(|m| m.display().to_string()).collect())
}

fn unescape_octal(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let code = (b[i] == b'\\').then(|| b.get(i + 1..i + 4)).flatten().and_then(|d| std::str::from_utf8(d).ok()).and_then(|d| u8::from_str_radix(d, 8).ok());
        if let Some(n) = code { out.push(n); i += 4; } else { out.push(b[i]); i += 1; }
    }
    out
}

/// Refused when something is mounted inside the folder.
fn none_mounted(dir: &Path) -> Res<()> {
    let mounted = mounts_inside(dir)?;
    if mounted.is_empty() { Ok(()) } else { Err(format!("nothing changed: something is mounted inside {} ({}); unmount it first", dir.display(), mounted.join(", "))) }
}

/// Deletes a project's snapshots and empties its folder but for its marker (the person confirmed). Stopped part way,
/// the same call goes on from there; `remove_rest` then takes the marker and the folder.
pub fn delete_all(dir: &Path) -> Res<()> {
    // Deleting a link would leave the data where it points.
    if fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) { return Err(format!("{} is a link: delete the folder it points to, not the link", dir.display())); }
    none_mounted(dir)?;
    let mine = owner(dir).ok_or_else(|| format!("{} holds no project's data", dir.display()))?;
    for name in numbered(dir, &mine, false).into_iter().chain(numbered(dir, &mine, true)) {
        delete_snapshot(&snapshot_dir(dir).join(&name)).map_err(|e| format!("snapshot {name} is left: {e}"))?;
    }
    drop_empty(dir);
    // Only when empty: snapshots of an earlier project of the same folder name are not this one's.
    let _ = fs::remove_dir(snapshot_dir(dir));
    empty_but_marker(dir)
}

/// The marker and the emptied folder, last.
pub fn remove_rest(dir: &Path) -> Res<()> { discard(dir, is_subvolume(dir)) }

/// Deletes the oldest snapshots past `limit`. Answers the names deleted; an error names those left.
/// Those left half deleted first: while one cannot be deleted no other is begun (begun, a snapshot is off the list for
/// good), so whatever stops it costs one snapshot, not one a save. At most 3 a time: a pile from before (more than 30)
/// goes down over saves, not in one long one.
fn prune(dir: &Path, limit: usize) -> Res<Vec<String>> {
    left_over(dir)?;
    let all = snapshots(dir);
    let extra = &all[..all.len().saturating_sub(limit)];
    let extra = &extra[..extra.len().min(PRUNE_AT_ONCE)];
    if extra.is_empty() { return Ok(Vec::new()); }
    let mut gone = Vec::new();
    for name in extra {
        delete_snapshot(&snapshot_dir(dir).join(name)).map_err(|e| format!("snapshot {name} is left half deleted: {e}"))?;
        gone.push(name.clone());
    }
    Ok(gone)
}

/// Programs working in the folder, as `fuser -m` sees them (a subvolume has a device of its own). Empty: none.
fn busy(dir: &Path) -> Res<String> {
    let out = Command::new("fuser").arg("-m").arg(dir).output().map_err(|e| format!("cannot check what uses the folder (fuser, of the package psmisc): {e}"))?;
    // fuser exits 0 with process ids when some use it, 1 when none do.
    match out.status.code() {
        Some(0) => Ok(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        Some(1) => Ok(String::new()),
        _ => Err(format!("cannot check what uses the folder (fuser): {}", String::from_utf8_lossy(&out.stderr).trim())),
    }
}

/// A subvolume inside the folder: a snapshot holds only an empty folder in its place, so restoring would lose it.
fn nested_subvolume(dir: &Path) -> Res<Option<String>> {
    // A folder find could not read may hold one: that is not taken for none.
    output("find", &[dir.as_os_str(), "-mindepth".as_ref(), "1".as_ref(), "-xdev".as_ref(), "-type".as_ref(), "d".as_ref(), "-inum".as_ref(), "256".as_ref(), "-print".as_ref(), "-quit".as_ref()])
        .map(|p| (!p.is_empty()).then_some(p)).map_err(|e| format!("cannot check the folder for subvolumes: {e}"))
}

/// A restore done, the snapshot of the state before it, and what could not be cleaned up after it.
pub struct Restored { pub before: String, pub warning: Option<String> }

/// Puts the folder back as it was at snapshot `name`. The state now is kept first as a snapshot of its own.
/// Nothing may be working in the folder: the host's conversation restores, after the project's ones stopped.
pub fn restore(dir: &Path, name: &str) -> Res<Restored> { restore_keeping(dir, name, SNAPSHOT_LIMIT) }

fn restore_keeping(dir: &Path, name: &str, limit: usize) -> Res<Restored> {
    let snap = snapshot_dir(dir).join(name);
    if !snapshots(dir).iter().any(|n| n == name) { return Err(format!("no snapshot {name}")); }
    let using = busy(dir)?;
    if !using.is_empty() {
        return Err(format!("programs work in the project folder (process ids {using}): stop them, then restore from outside the folder"));
    }
    if let Some(inner) = nested_subvolume(dir)? { return Err(format!("a subvolume is inside the project folder ({inner}): a snapshot does not hold its contents, so it is not restored")); }
    // The old folder is deleted after the swap: file by file it would go into what is mounted there.
    none_mounted(dir)?;
    // Pruned only after: at the limit, the snapshot to go back to can be the oldest.
    let before = take(dir)?;
    let folder = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stamp = now_ms();
    let (aside, fresh) = (dir.with_file_name(format!(".{folder}.restoring-{stamp}")), dir.with_file_name(format!(".{folder}.restored-{stamp}")));
    // Made beside first, then put in place by a rename that replaces nothing: a path made again meanwhile is not
    // filled with it.
    // btrfs puts a snapshot inside a folder already there: that name must be free, and what is made the subvolume.
    if fs::symlink_metadata(&fresh).is_ok() { return Err(format!("{} is in the way: restore again", fresh.display())); }
    btrfs(&["subvolume".as_ref(), "snapshot".as_ref(), snap.as_os_str(), fresh.as_os_str()])?;
    if !is_subvolume(&fresh) { return Err(format!("{} is not the restored snapshot; nothing was changed", fresh.display())); }
    // A vault keeps the key file it has now, put in before the swap: its key (in the keyring) may have been changed
    // since the snapshot, and the data's own key, the recovery key, never changes, so today's file opens the restored
    // data. Not put in: nothing is swapped. (gocryptfs leaves it read-only: replaced, not written over.)
    let conf = Path::new(wb_core::roots::PROJECT_DATA).join("gocryptfs.conf");
    // Only a vault's (the snapshot has one too), and only a plain file: never through a link to somewhere else.
    let plain = |p: &Path| fs::symlink_metadata(p).is_ok_and(|m| m.file_type().is_file())
        && fs::symlink_metadata(p.parent().unwrap_or(p)).is_ok_and(|m| m.file_type().is_dir());
    if plain(&dir.join(&conf)) && plain(&fresh.join(&conf)) {
        let put = fs::remove_file(fresh.join(&conf)).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }).and_then(|_| fs::copy(dir.join(&conf), fresh.join(&conf)));
        if let Err(e) = put { let _ = discard(&fresh, true); return Err(format!("the vault's key file could not be kept: {e}; nothing was changed")); }
    }
    if let Err(e) = rename_new(dir, &aside) { let _ = discard(&fresh, true); return Err(e); }
    if let Err(e) = rename_new(&fresh, dir) {
        let back = rename_new(&aside, dir);
        let _ = discard(&fresh, true);
        return Err(match back { Ok(()) => e, Err(e2) => format!("{e}; and the folder could not be put back from {}: {e2}", aside.display()) });
    }
    let mut warnings = Vec::new();
    // The state before is in snapshot `before`, so the copy set aside can go.
    if let Err(e) = discard(&aside, true) { warnings.push(format!("the old folder is left at {}: {e}", aside.display())); }
    if let Err(e) = prune(dir, limit) { warnings.push(e); }
    Ok(Restored { before, warning: (!warnings.is_empty()).then(|| warnings.join("; ")) })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A place in the build folder: btrfs on a developer's machine, a plain file system in CI. Both ways are checked.
    fn place(name: &str) -> PathBuf {
        let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/test-folders").join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn mounts_inside_a_folder_are_found() {
        // /proc itself is a mount inside /, not / itself.
        assert!(mounts_inside(Path::new("/")).unwrap().iter().any(|m| m == "/proc"));
        assert!(!mounts_inside(Path::new("/proc")).unwrap().iter().any(|m| m == "/proc"), "the folder itself is not inside");
        // A space as octal, and bytes that are not UTF-8 kept as they are.
        assert_eq!(unescape_octal(b"/srv/a\\040b/\xff"), b"/srv/a b/\xff");
        assert!(none_mounted(Path::new("/")).is_err() && none_mounted(Path::new("/proc/self/fd")).is_ok());
    }

    #[test]
    fn a_deleted_folder_goes_with_its_snapshots_only() {
        let root = place("delete");
        let dir = root.join("proj");
        let subvolume = make(&dir, None).unwrap();
        fs::create_dir_all(dir.join(".whaybench")).unwrap();
        fs::write(dir.join(".whaybench/briefsafe.json"), r#"{"id":"t","made":"1"}"#).unwrap();
        // A read-only folder inside (Go's module cache): emptied all the same.
        fs::create_dir_all(dir.join("mod/x")).unwrap();
        fs::write(dir.join("mod/x/f"), "a").unwrap();
        run("chmod", &["-R".as_ref(), "a-w".as_ref(), dir.join("mod").as_os_str()]).unwrap();

        if subvolume {
            snapshot(&dir).unwrap();
            snapshot(&dir).unwrap();
            assert_eq!(snapshots(&dir).len(), 2);
            // Pruned file by file where the mount option is not, read-only folder and all; 3 at most a time.
            for _ in 0..3 { snapshot(&dir).unwrap(); }
            assert_eq!(snapshots(&dir).len(), 5);
            assert_eq!(prune(&dir, 1).unwrap().len(), 3);
            assert_eq!(prune(&dir, 1).unwrap().len(), 1);
            // Another project's snapshot beside it stays, read-only or writable (a subvolume made writable here).
            let other = snapshot_dir(&dir).join("7");
            btrfs(&["subvolume".as_ref(), "create".as_ref(), other.as_os_str()]).unwrap();
            fs::create_dir_all(other.join(".whaybench")).unwrap();
            fs::write(other.join(".whaybench/briefsafe.json"), r#"{"id":"t","made":"2"}"#).unwrap();
        }
        assert!(!size(&dir).is_empty());
        // A link is refused, the folder it points to untouched.
        let link = root.join("link");
        std::os::unix::fs::symlink(&dir, &link).unwrap();
        assert!(delete_all(&link).is_err());
        assert!(dir.exists());
        if subvolume {
            // A delete stopped part way: one snapshot made writable (off the list) and half emptied.
            let half = snapshot_dir(&dir).join(&snapshots(&dir)[0]);
            btrfs(&["property".as_ref(), "set".as_ref(), "-ts".as_ref(), half.as_os_str(), "ro".as_ref(), "false".as_ref()]).unwrap();
            run("chmod", &["-R".as_ref(), "u+w".as_ref(), half.join("mod").as_os_str()]).unwrap();
            fs::remove_dir_all(half.join("mod")).unwrap();
            assert!(snapshots(&dir).is_empty(), "never listed or restored half emptied");
            assert_eq!(numbered(&dir, &owner(&dir).unwrap(), false).len(), 1);
            // The next save deletes it.
            snapshot(&dir).unwrap();
            assert!(numbered(&dir, &owner(&dir).unwrap(), false).is_empty() && !half.exists());
            // An empty one left by a delete stopped after its marker went: the delete takes it.
            btrfs(&["subvolume".as_ref(), "create".as_ref(), snapshot_dir(&dir).join("8").as_os_str()]).unwrap();
        }
        // One the owner cannot even read or enter (no snapshot is taken of such a folder: find cannot look in it).
        fs::create_dir_all(dir.join("shut/in")).unwrap();
        fs::write(dir.join("shut/in/f"), "a").unwrap();
        run("chmod", &["000".as_ref(), dir.join("shut").as_os_str()]).unwrap();
        // Emptied but for the marker; a try stopped part way (something left) goes on, then the rest goes.
        delete_all(&dir).unwrap();
        assert!(owner(&dir).is_some() && fs::read_dir(&dir).unwrap().count() == 1 && fs::read_dir(dir.join(".whaybench")).unwrap().count() == 1);
        fs::write(dir.join("left.txt"), "x").unwrap();
        fs::write(dir.join(".whaybench/left"), "x").unwrap();
        delete_all(&dir).unwrap();
        assert!(fs::read_dir(&dir).unwrap().count() == 1 && fs::read_dir(dir.join(".whaybench")).unwrap().count() == 1);
        let mine = owner(&dir).unwrap();
        assert!(numbered(&dir, &mine, true).is_empty() && numbered(&dir, &mine, false).is_empty(), "no snapshot of it left");
        remove_rest(&dir).unwrap();
        assert!(!dir.exists());
        if subvolume {
            assert!(snapshot_dir(&dir).join("7/.whaybench/briefsafe.json").exists() && !snapshot_dir(&dir).join("8").exists());
            discard(&snapshot_dir(&dir).join("7"), true).unwrap();
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_folder_is_made_copied_snapshotted_pruned_and_restored() {
        let root = place("all");
        let src = root.join("old");
        fs::create_dir_all(src.join("src")).unwrap();
        fs::write(src.join("src/main.rs"), "fn main() {}").unwrap();
        assert!(make(&src.join("inner"), Some(&src)).is_err(), "not inside the folder copied from");
        assert!(make(&root.join("x"), Some(&root.join("missing"))).is_err());
        let dir = root.join("proj");
        let subvolume = make(&dir, Some(&src)).unwrap();
        assert_eq!(subvolume, on_btrfs(&root));
        assert_eq!(fs::read_to_string(dir.join("src/main.rs")).unwrap(), "fn main() {}");
        assert!(src.join("src/main.rs").exists(), "the source stays");
        assert!(make(&dir, None).is_err(), "an existing folder is not made again");
        if !subvolume {
            assert!(snapshot(&dir).is_err(), "no snapshots without btrfs");
            fs::remove_dir_all(&root).unwrap();
            return;
        }
        eprintln!("btrfs: snapshots checked");
        // A project's folder: its marker says whose its snapshots are.
        let mark = |id: &str| { fs::create_dir_all(dir.join(".whaybench")).unwrap(); fs::write(dir.join(".whaybench/briefsafe.json"), format!(r#"{{"id":"{id}","made":"1"}}"#)).unwrap(); };
        mark("t");
        // A vault: its key file is in every snapshot.
        fs::write(dir.join(".whaybench/gocryptfs.conf"), "then").unwrap();
        let first = snapshot(&dir).unwrap().name;
        fs::write(dir.join("src/main.rs"), "changed").unwrap();
        let second = snapshot(&dir).unwrap().name;
        assert!(second.parse::<u64>().unwrap() > first.parse::<u64>().unwrap());
        // A link with a number for a name is not a snapshot, so pruning cannot follow it.
        std::os::unix::fs::symlink(&src, snapshot_dir(&dir).join("1")).unwrap();
        assert_eq!(snapshots(&dir), vec![first.clone(), second.clone()]);
        assert_eq!(fs::read_to_string(snapshot_dir(&dir).join(&first).join("src/main.rs")).unwrap(), "fn main() {}");

        // A vault's key file as it is now stays (its key may have changed since the snapshot).
        let _ = fs::remove_file(dir.join(".whaybench/gocryptfs.conf"));
        fs::write(dir.join(".whaybench/gocryptfs.conf"), "now").unwrap();
        let before = restore(&dir, &first).unwrap().before;
        assert_eq!(fs::read_to_string(dir.join(".whaybench/gocryptfs.conf")).unwrap(), "now");
        assert_eq!(fs::read_to_string(dir.join("src/main.rs")).unwrap(), "fn main() {}");
        assert_eq!(fs::read_to_string(snapshot_dir(&dir).join(&before).join("src/main.rs")).unwrap(), "changed", "the state before is kept");
        fs::write(dir.join("new"), "x").unwrap();
        assert!(restore(&dir, "1").is_err(), "a link is not a snapshot");
        // Snapshots of another project left beside a folder of the same name are not this one's, nor any of a folder
        // that is no project's.
        mark("up");
        assert!(snapshots(&dir).is_empty() && restore(&dir, &first).is_err());
        fs::remove_dir_all(dir.join(".whaybench")).unwrap();
        assert!(snapshots(&dir).is_empty());
        mark("t");
        // A writable subvolume with a number for a name is not a snapshot.
        btrfs(&["subvolume".as_ref(), "snapshot".as_ref(), dir.as_os_str(), snapshot_dir(&dir).join("5").as_os_str()]).unwrap();
        assert!(!snapshots(&dir).contains(&"5".to_string()));
        discard(&snapshot_dir(&dir).join("5"), true).unwrap();
        // A subvolume inside: no snapshot that would miss it.
        btrfs(&["subvolume".as_ref(), "create".as_ref(), dir.join("inner").as_os_str()]).unwrap();
        assert!(snapshot(&dir).is_err());
        discard(&dir.join("inner"), true).unwrap();
        // At the limit, going back to the oldest still works: it is pruned only after.
        let r = restore_keeping(&dir, &first, 3).unwrap();
        assert!(!dir.join("new").exists());
        // Pruned with or without user_subvol_rm_allowed (without it, emptied like a folder).
        assert!(r.warning.is_none(), "{:?}", r.warning);
        assert_eq!(snapshots(&dir), vec![second.clone(), before.clone(), r.before.clone()]);
        assert_eq!(prune(&dir, 1).unwrap(), vec![second, before]);
        assert_eq!(snapshots(&dir), vec![r.before]);
        prune(&dir, 0).unwrap();
        discard(&dir, true).unwrap();
        assert!(src.join("src/main.rs").exists(), "the linked folder is untouched");
        let _ = fs::remove_dir_all(&root);
    }
}

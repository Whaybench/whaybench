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

fn is_subvolume(path: &Path) -> bool { fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && m.ino() == SUBVOLUME_INODE) }

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
    let subvolume = on_btrfs(parent);
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
    fs::remove_dir_all(dir).map_err(|e| e.to_string())
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

fn snapshot_dir(dir: &Path) -> PathBuf {
    let name = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    dir.with_file_name(format!(".{name}.snapshots"))
}

/// The snapshots of a project folder, oldest first: subvolumes with a number for a name, never a link, of this
/// project (one left beside a folder of the same name by an earlier project is not this one's to restore or prune).
pub fn snapshots(dir: &Path) -> Vec<String> {
    let at = snapshot_dir(dir);
    if !fs::symlink_metadata(&at).is_ok_and(|m| m.is_dir()) { return Vec::new(); }
    // A folder of no project has no snapshots of its own to tell apart; only read-only ones are snapshots.
    let Some(mine) = owner(dir) else { return Vec::new() };
    let mut names: Vec<String> = fs::read_dir(&at).into_iter().flatten().flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| n.parse::<u64>().is_ok() && is_subvolume(&at.join(n)) && read_only(&at.join(n)) && owner(&at.join(n)).as_ref() == Some(&mine)).collect();
    names.sort_by_key(|n| n.parse::<u64>().unwrap_or(0));
    names
}

/// A snapshot taken, and what could not be cleaned up after it (the step itself worked).
pub struct Taken { pub name: String, pub warning: Option<String> }

/// Takes a read-only snapshot, then keeps no more than 30.
pub fn snapshot(dir: &Path) -> Res<Taken> {
    let name = take(dir)?;
    Ok(Taken { name, warning: prune(dir, SNAPSHOT_LIMIT).err() })
}

fn take(dir: &Path) -> Res<String> {
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

/// Whether a user may delete subvolumes here: the file system is mounted `user_subvol_rm_allowed`.
fn may_delete(path: &Path) -> bool {
    output("findmnt", &["-n".as_ref(), "-o".as_ref(), "OPTIONS".as_ref(), "-T".as_ref(), path.as_os_str()])
        .is_ok_and(|o| o.split(',').any(|x| x == "user_subvol_rm_allowed"))
}

const NOT_ALLOWED: &str = "snapshots past the limit are kept: deleting them needs the file system mounted with user_subvol_rm_allowed";

/// A read-only snapshot is made writable first (its owner may), and only then may a user delete it. If the delete
/// fails it is made read-only again, so a kept snapshot stays safe.
fn delete_snapshot(path: &Path) -> Res<()> {
    let ro = |v: &str| btrfs(&["property".as_ref(), "set".as_ref(), "-ts".as_ref(), path.as_os_str(), "ro".as_ref(), v.as_ref()]);
    ro("false")?;
    btrfs(&["subvolume".as_ref(), "delete".as_ref(), path.as_os_str()]).inspect_err(|_| { let _ = ro("true"); })
}

/// Deletes the oldest snapshots past `limit`. Answers the names deleted; an error names those left.
fn prune(dir: &Path, limit: usize) -> Res<Vec<String>> {
    let all = snapshots(dir);
    let extra = &all[..all.len().saturating_sub(limit)];
    if extra.is_empty() { return Ok(Vec::new()); }
    if !may_delete(dir) { return Err(format!("{NOT_ALLOWED} ({})", extra.join(", "))); }
    let mut gone = Vec::new();
    for name in extra {
        delete_snapshot(&snapshot_dir(dir).join(name)).map_err(|e| format!("snapshot {name} kept: {e}"))?;
        gone.push(name.clone());
    }
    Ok(gone)
}

/// Programs working in the folder, as `fuser -m` sees them (a subvolume has a device of its own). Empty: none.
fn busy(dir: &Path) -> Res<String> {
    let out = Command::new("fuser").arg("-m").arg(dir).output().map_err(|e| format!("cannot check what uses the folder (fuser): {e}"))?;
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
    // Pruned only after: at the limit, the snapshot to go back to can be the oldest.
    let before = take(dir)?;
    let folder = dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let stamp = now_ms();
    let (aside, fresh) = (dir.with_file_name(format!(".{folder}.restoring-{stamp}")), dir.with_file_name(format!(".{folder}.restored-{stamp}")));
    // Made beside first, then put in place by a rename that replaces nothing: a path made again meanwhile is not
    // filled with it.
    btrfs(&["subvolume".as_ref(), "snapshot".as_ref(), snap.as_os_str(), fresh.as_os_str()])?;
    if let Err(e) = rename_new(dir, &aside) { let _ = discard(&fresh, true); return Err(e); }
    if let Err(e) = rename_new(&fresh, dir) {
        let back = rename_new(&aside, dir);
        let _ = discard(&fresh, true);
        return Err(match back { Ok(()) => e, Err(e2) => format!("{e}; and the folder could not be put back from {}: {e2}", aside.display()) });
    }
    // The state before is in snapshot `before`, so the copy set aside can go.
    let mut warnings = Vec::new();
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
        let first = snapshot(&dir).unwrap().name;
        fs::write(dir.join("src/main.rs"), "changed").unwrap();
        let second = snapshot(&dir).unwrap().name;
        assert!(second.parse::<u64>().unwrap() > first.parse::<u64>().unwrap());
        // A link with a number for a name is not a snapshot, so pruning cannot follow it.
        std::os::unix::fs::symlink(&src, snapshot_dir(&dir).join("1")).unwrap();
        assert_eq!(snapshots(&dir), vec![first.clone(), second.clone()]);
        assert_eq!(fs::read_to_string(snapshot_dir(&dir).join(&first).join("src/main.rs")).unwrap(), "fn main() {}");

        let before = restore(&dir, &first).unwrap().before;
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
        if may_delete(&dir) {
            assert!(r.warning.is_none(), "{:?}", r.warning);
            assert_eq!(snapshots(&dir), vec![second.clone(), before.clone(), r.before.clone()]);
            assert_eq!(prune(&dir, 1).unwrap(), vec![second, before]);
            assert_eq!(snapshots(&dir), vec![r.before]);
            prune(&dir, 0).unwrap();
            discard(&dir, true).unwrap();
        } else {
            assert!(prune(&dir, 1).is_err(), "kept, and said so");
        }
        assert!(src.join("src/main.rs").exists(), "the linked folder is untouched");
        let _ = fs::remove_dir_all(&root);
    }
}

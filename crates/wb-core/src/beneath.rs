//! Files in an open Briefsafe project's data root (`<host data>/cache/open/<id>`), as host programs touch them (spec
//! 3.22). The project's agents can put anything there: links to the host's files, named pipes, files of any size.
//! So everything is opened beneath the root (openat2), never through a link, without blocking, and read only up to a
//! limit; a write makes a new file of its own and renames it into place.
use std::{
    ffi::CString,
    io::{self, Error, ErrorKind, Read, Write},
    os::{fd::{AsRawFd, FromRawFd, OwnedFd}, unix::ffi::OsStrExt},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// What host programs read from a project at most, per file.
pub const READ_LIMIT: u64 = 4 << 20;

/// The open project root `path` is in, and the path below it; None for a path in no project's open place.
pub fn split(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let root = path.ancestors().find(|a| a.parent().is_some_and(|p| p.ends_with("cache/open")))?;
    Some((root.to_path_buf(), path.strip_prefix(root).ok()?.to_path_buf()))
}

fn c(path: &Path) -> io::Result<CString> { CString::new(path.as_os_str().as_bytes()).map_err(|e| Error::new(ErrorKind::InvalidInput, e)) }

/// The root itself is the host's own link to where the project's data rests: it is followed, nothing below it is.
fn root_fd(root: &Path) -> io::Result<OwnedFd> {
    // Safety: a plain open; the descriptor is owned right away.
    let fd = unsafe { libc::open(c(root)?.as_ptr(), libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC) };
    if fd < 0 { Err(Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd) }) }
}

fn at(dir: &OwnedFd, rel: &Path, flags: i32, mode: u32) -> io::Result<OwnedFd> {
    let path = c(rel)?;
    // Safety: open_how is plain data, zeroed then set; the kernel reads `size` bytes of it.
    let mut how: libc::open_how = unsafe { std::mem::zeroed() };
    how.flags = (flags | libc::O_CLOEXEC | libc::O_NOFOLLOW) as u64;
    how.mode = if flags & libc::O_CREAT != 0 { mode as u64 } else { 0 };
    how.resolve = libc::RESOLVE_BENEATH | libc::RESOLVE_NO_SYMLINKS | libc::RESOLVE_NO_MAGICLINKS;
    let fd = unsafe { libc::syscall(libc::SYS_openat2, dir.as_raw_fd(), path.as_ptr(), &how as *const libc::open_how, std::mem::size_of::<libc::open_how>()) };
    if fd < 0 { Err(Error::last_os_error()) } else { Ok(unsafe { OwnedFd::from_raw_fd(fd as i32) }) }
}

/// Opens (making what is missing) each folder of `rel` in turn from `dir`, by the folder before it: a folder made is
/// always made in one already checked, never through a link put on the way.
fn made_dirs(dir: OwnedFd, rel: &Path) -> io::Result<OwnedFd> {
    let mut at_dir = dir;
    for part in rel.components() {
        let std::path::Component::Normal(name) = part else { return Err(Error::new(ErrorKind::InvalidInput, "not a plain path")) };
        let n = c(Path::new(name))?;
        // Safety: mkdirat with a single name in a folder this function holds open.
        if unsafe { libc::mkdirat(at_dir.as_raw_fd(), n.as_ptr(), 0o700) } != 0 && Error::last_os_error().kind() != ErrorKind::AlreadyExists { return Err(Error::last_os_error()); }
        at_dir = at(&at_dir, Path::new(name), libc::O_RDONLY | libc::O_DIRECTORY, 0)?;
    }
    Ok(at_dir)
}

fn dir_of(root: &Path, rel: &Path) -> io::Result<OwnedFd> {
    let base = root_fd(root)?;
    if rel.as_os_str().is_empty() { return Ok(base); }
    at(&base, rel, libc::O_RDONLY | libc::O_DIRECTORY, 0)
}

/// A regular file's bytes, up to `max`. A link, a pipe or a device is refused; so is a file over the limit.
pub fn read(root: &Path, rel: &Path, max: u64) -> io::Result<Vec<u8>> {
    let fd = at(&root_fd(root)?, rel, libc::O_RDONLY | libc::O_NONBLOCK, 0)?;
    let file = std::fs::File::from(fd);
    if !file.metadata()?.is_file() { return Err(Error::new(ErrorKind::InvalidData, "not a regular file")); }
    let mut bytes = Vec::new();
    file.take(max + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > max { return Err(Error::new(ErrorKind::InvalidData, "file too large")); }
    Ok(bytes)
}

/// The names in a folder beneath the root (the folder itself not a link).
pub fn list(root: &Path, rel: &Path) -> io::Result<Vec<String>> {
    let fd = dir_of(root, rel)?;
    // The opened folder, read by its descriptor: what is listed is that folder, whatever its path now points to.
    let names = std::fs::read_dir(format!("/proc/self/fd/{}", fd.as_raw_fd()))?.flatten().filter_map(|e| e.file_name().into_string().ok()).collect();
    drop(fd);
    Ok(names)
}

/// Removes one entry (a file or a link itself, never what a link points to).
pub fn remove(root: &Path, rel: &Path) -> io::Result<()> {
    let (parent, name) = (rel.parent().unwrap_or(Path::new("")), rel.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?);
    let dir = dir_of(root, parent)?;
    // Safety: unlinkat on a descriptor this function owns, with a name that has no slash.
    if unsafe { libc::unlinkat(dir.as_raw_fd(), c(Path::new(name))?.as_ptr(), 0) } != 0 { return Err(Error::last_os_error()); }
    Ok(())
}

/// Writes a whole file beneath the root: missing folders are made (none through a link), the bytes go to a new file
/// of a name nobody can guess, which is then renamed over the old one (a link there is replaced, not followed).
pub fn write(root: &Path, rel: &Path, bytes: &[u8]) -> io::Result<()> {
    static N: AtomicU64 = AtomicU64::new(0);
    let (parent, name) = (rel.parent().unwrap_or(Path::new("")), rel.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?);
    let dir = made_dirs(root_fd(root)?, parent)?;
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let tmp = format!(".{}.{}-{}-{}.tmp", name.to_string_lossy(), std::process::id(), stamp, N.fetch_add(1, Ordering::Relaxed));
    let fd = at(&dir, Path::new(&tmp), libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL, 0o600)?;
    let mut file = std::fs::File::from(fd);
    let done = file.write_all(bytes).and_then(|_| {
        // Safety: renameat between names in the same owned folder descriptor.
        if unsafe { libc::renameat(dir.as_raw_fd(), c(Path::new(&tmp))?.as_ptr(), dir.as_raw_fd(), c(Path::new(name))?.as_ptr()) } != 0 { Err(Error::last_os_error()) } else { Ok(()) }
    });
    if done.is_err() { unsafe { libc::unlinkat(dir.as_raw_fd(), c(Path::new(&tmp))?.as_ptr(), 0) }; }
    done
}

/// Makes a folder (and those above it) beneath the root, none through a link.
pub fn mkdir(root: &Path, rel: &Path) -> io::Result<()> { made_dirs(root_fd(root)?, rel).map(|_| ()) }

/// Puts a link at `rel` beneath the root (in place of what was there), pointing to `target`.
pub fn link(root: &Path, rel: &Path, target: &Path) -> io::Result<()> {
    let (parent, name) = (rel.parent().unwrap_or(Path::new("")), rel.file_name().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no name"))?);
    if !parent.as_os_str().is_empty() { mkdir(root, parent)?; }
    let dir = dir_of(root, parent)?;
    let n = c(Path::new(name))?;
    // Safety: unlinkat and symlinkat on a descriptor this function owns; a name without a slash.
    unsafe { libc::unlinkat(dir.as_raw_fd(), n.as_ptr(), 0) };
    if unsafe { libc::symlinkat(c(target)?.as_ptr(), dir.as_raw_fd(), n.as_ptr()) } != 0 { return Err(Error::last_os_error()); }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn nothing_beneath_a_project_root_leads_out_of_it_or_blocks() {
        let base = std::env::temp_dir().join(format!("wb-beneath-{}", std::process::id()));
        let _ = fs::remove_dir_all(&base);
        let (rest, outside) = (base.join("rest"), base.join("outside"));
        fs::create_dir_all(rest.join("cache/ctl")).unwrap();
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("secret.json"), "host").unwrap();
        fs::create_dir_all(base.join("host/cache/open")).unwrap();
        let root = base.join("host/cache/open/up");
        std::os::unix::fs::symlink(&rest, &root).unwrap();
        assert_eq!(split(&root.join("sessions/a.json")), Some((root.clone(), PathBuf::from("sessions/a.json"))));
        assert_eq!(split(&base.join("host/sessions/a.json")), None);

        fs::write(rest.join("cache/ctl/a.json"), "{}").unwrap();
        assert_eq!(read(&root, Path::new("cache/ctl/a.json"), 10).unwrap(), b"{}");
        assert!(read(&root, Path::new("cache/ctl/a.json"), 1).is_err(), "over the limit");
        // A link planted by the agent, to a file or a folder outside: not followed.
        std::os::unix::fs::symlink(outside.join("secret.json"), rest.join("cache/ctl/b.json")).unwrap();
        assert!(read(&root, Path::new("cache/ctl/b.json"), 100).is_err());
        std::os::unix::fs::symlink(&outside, rest.join("ask")).unwrap();
        assert!(list(&root, Path::new("ask")).is_err());
        assert!(write(&root, Path::new("ask/secret.json"), b"x").is_err());
        // Nor is a folder made on the far side of a link on the way.
        assert!(mkdir(&root, Path::new("ask/made/deeper")).is_err() && write(&root, Path::new("ask/made/x.json"), b"x").is_err());
        assert!(!outside.join("made").exists(), "nothing made outside");
        assert_eq!(fs::read_to_string(outside.join("secret.json")).unwrap(), "host");
        // A named pipe does not block the reader.
        let fifo = rest.join("cache/ctl/c.json");
        assert_eq!(unsafe { libc::mkfifo(c(&fifo).unwrap().as_ptr(), 0o600) }, 0);
        assert!(read(&root, Path::new("cache/ctl/c.json"), 100).is_err());
        // Writing over a planted link replaces the link; the file it pointed to is untouched.
        write(&root, Path::new("cache/ctl/b.json"), b"new").unwrap();
        assert_eq!(fs::read_to_string(rest.join("cache/ctl/b.json")).unwrap(), "new");
        assert_eq!(fs::read_to_string(outside.join("secret.json")).unwrap(), "host");
        write(&root, Path::new("view/history/s-a/index.json"), b"[]").unwrap();
        let mut names = list(&root, Path::new("cache/ctl")).unwrap();
        names.sort();
        assert_eq!(names, vec!["a.json", "b.json", "c.json"]);
        remove(&root, Path::new("cache/ctl/c.json")).unwrap();
        remove(&root, Path::new("ask")).unwrap();
        assert!(outside.exists(), "removing a link leaves what it pointed to");
        fs::remove_dir_all(&base).unwrap();
    }
}

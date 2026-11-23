//! Secrets kept in the person's keyring through the Secret Service (spec 3.22): KWallet, GNOME Keyring, oo7 or
//! KeePassXC, whichever runs. Nothing secret is written to a file or a log. A locked keyring can wait on a prompt
//! nobody sees, so every call has a time limit.
use std::{collections::HashMap, time::Duration};

/// The long-lived Claude Code token (`claude setup-token`) the project containers' agents sign in with.
pub const CLAUDE_TOKEN: &str = "claude-token";
/// How the keyring's own tools name it.
pub const CLAUDE_TOKEN_LABEL: &str = "Whaybench: Claude Code token for project containers";

/// Whether text looks like a token from `claude setup-token`: `sk-ant-`, then only letters, digits, `-` and `_`.
pub fn claude_token_shaped(t: &str) -> bool {
    t.len() > 20 && t.starts_with("sk-ant-") && t.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

const LIMIT: Duration = Duration::from_secs(15);

fn attributes(name: &str) -> HashMap<&str, &str> { HashMap::from([("application", "whaybench"), ("secret", name)]) }

fn run<T>(work: impl std::future::Future<Output = Result<T, oo7::Error>>) -> Result<T, String> { run_for(LIMIT, work) }

fn run_for<T>(limit: Duration, work: impl std::future::Future<Output = Result<T, oo7::Error>>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async { tokio::time::timeout(limit, work).await })
        .map_err(|_| SILENT.to_string())?
        .map_err(|e| format!("keyring: {e}"))
}

/// How the keyring is now, asked without any prompt.
#[derive(Debug, PartialEq)]
pub enum State { Open, Locked, /** The service runs, with no keyring made yet. */ New, /** There, and it did not answer in time. */ Silent(String), Absent(String) }

const SILENT: &str = "the keyring did not answer (locked, waiting for its password?)";

pub fn state() -> State {
    let asked = run(async {
        let service = oo7::dbus::Service::new().await?;
        // (`default_collection` would make one, with a prompt.)
        Ok(match service.with_alias(oo7::dbus::Service::DEFAULT_COLLECTION).await? {
            Some(c) => if c.is_locked().await? { State::Locked } else { State::Open },
            None => State::New,
        })
    });
    // Slow is not missing: what can be had in a moment must not be done without.
    asked.unwrap_or_else(|e| if e == SILENT { State::Silent(e) } else { State::Absent(e) })
}

/// Where the keyring's own password window cannot be relied on (WSL has no desktop to bring one): its password is
/// asked in Whaybench and given to gnome-keyring by `unlock_with`.
pub fn asks_here() -> bool { crate::input::wsl() && crate::versions::find_program("gnome-keyring-daemon", &std::env::var("PATH").unwrap_or_default(), None).is_some() }

/// What a step says when the keyring's password must be given first (Whaybench then asks it: `asks_here`).
pub const LOCKED: &str = "the keyring is locked";
pub const NOT_MADE: &str = "the keyring has not been made yet";

/// The keyring open for what follows: waits for the person at the keyring's own window (`waiting` is called first), or
/// answers LOCKED or NOT_MADE where Whaybench asks the password itself.
pub fn ensure_open(limit: Duration, waiting: impl FnOnce()) -> Result<(), String> {
    match state() {
        State::Open => return Ok(()),
        State::Absent(e) | State::Silent(e) => return Err(e),
        // (One with no password opens without asking.)
        State::Locked if asks_here() => return if unlock_quietly().is_ok_and(|open| open) { Ok(()) } else { Err(LOCKED.into()) },
        State::New if asks_here() => return Err(NOT_MADE.into()),
        _ => waiting(),
    }
    run_for(limit, async { oo7::Keyring::new().await?.unlock().await })
}

use oo7::zbus::{Connection, Error as BusError, OwnedObjectPath, OwnedVariant, Variant};

const SERVICE: &str = "org.freedesktop.secrets";
const AT: &str = "/org/freedesktop/secrets";
const API: &str = "org.freedesktop.Secret.Service";

fn bus<T>(work: impl std::future::Future<Output = Result<T, BusError>>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async { tokio::time::timeout(LIMIT, work).await }).map_err(|_| SILENT.to_string())?.map_err(|e| format!("keyring: {e}"))
}

/// The keyring used when none is named.
async fn default_path(c: &Connection) -> Result<OwnedObjectPath, BusError> {
    c.call_method(Some(SERVICE), AT, Some(API), "ReadAlias", &("default",)).await?.body().deserialize()
}

/// Opens the default keyring if it opens without a prompt (it has no password); a prompt it would show is not shown.
fn unlock_quietly() -> Result<bool, String> {
    bus(async {
        let c = Connection::session().await?;
        let path = default_path(&c).await?;
        let (open, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = c.call_method(Some(SERVICE), AT, Some(API), "Unlock", &(vec![path],)).await?.body().deserialize()?;
        if prompt.as_str() != "/" { let _ = c.call_method(Some(SERVICE), prompt.as_str(), Some("org.freedesktop.Secret.Prompt"), "Dismiss", &()).await; }
        Ok(!open.is_empty())
    })
}

/// Opens a keyring other than `login` with its password, as GNOME's own tools do (`gnome-keyring-daemon --unlock` knows
/// `login` only).
fn unlock_path(path: &str, password: &str) -> Result<(), String> {
    bus(async {
        let c = Connection::session().await?;
        let (_, session): (OwnedVariant, OwnedObjectPath) = c.call_method(Some(SERVICE), AT, Some(API), "OpenSession", &("plain", Variant::from(""))).await?.body().deserialize()?;
        let path = OwnedObjectPath::try_from(path)?;
        c.call_method(Some(SERVICE), AT, Some("org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface"), "UnlockWithMasterPassword",
            &(path, (session, Vec::<u8>::new(), password.as_bytes(), "text/plain"))).await?;
        Ok(())
    })
}

/// The file name (without its ending) of the keyring at this path: its last part, with what is not a letter or a digit
/// written `_xx` there.
fn keyring_file(path: &str) -> String {
    let last = path.rsplit('/').next().unwrap_or_default();
    let (mut out, mut rest) = (Vec::new(), last.as_bytes());
    while let Some((&c, tail)) = rest.split_first() {
        match tail.get(..2).filter(|_| c == b'_').and_then(|h| u8::from_str_radix(std::str::from_utf8(h).ok()?, 16).ok()) {
            Some(b) => { out.push(b); rest = &tail[2..]; }
            None => { out.push(c); rest = tail; }
        }
    }
    String::from_utf8_lossy(&out).replace('/', "_")
}

/// Opens the default keyring with its password, or makes gnome-keyring's login one when there is none. `anew`: the one
/// there (its password lost) is set aside first; what it held stays in that file, unreadable without its password.
pub fn unlock_with(password: &str, anew: bool) -> Result<(), String> {
    use std::{io::Write, process::{Command, Stdio}};
    if password.is_empty() { return Err("no password".into()); }
    let path = bus(async { default_path(&Connection::session().await?).await }).map(|p| p.to_string()).unwrap_or_default();
    let name = if path.len() > 1 { keyring_file(&path) } else { "login".into() };
    if !anew && name != "login" && state() == State::Locked {
        // A wrong password is an answer; what cannot be asked at all (another program's keyring) is said.
        if let Err(e) = unlock_path(&path, password) && (e == SILENT || e.contains("UnknownMethod") || e.contains("UnknownInterface")) { return Err(e); }
        return if state() == State::Open { Ok(()) } else { Err(LOCKED.into()) };
    }
    if anew && state() != State::New {
        let home = std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()).map(std::path::PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))).ok_or("no home folder")?;
        // And the note of which keyring is the default one: the login one is, without it.
        for file in [format!("{name}.keyring"), "default".into()].map(|f| home.join("keyrings").join(f)) {
            if file.exists() { std::fs::rename(&file, file.with_file_name(format!("{}.{}", file.file_name().unwrap_or_default().to_string_lossy(), crate::clock::now_ms()))).map_err(|e| e.to_string())?; }
        }
    }
    // On standard input only, without a newline (it would be part of the password).
    let mut child = Command::new("gnome-keyring-daemon").args(["--replace", "--unlock", "--components=secrets"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e| format!("gnome-keyring-daemon: {e}"))?;
    child.stdin.take().ok_or("no input")?.write_all(password.as_bytes()).map_err(|e| e.to_string())?;
    child.wait().map_err(|e| e.to_string())?;
    // It takes the place of the one before in a moment.
    for _ in 0..20 {
        if state() == State::Open { return Ok(()); }
        std::thread::sleep(Duration::from_millis(150));
    }
    Err(LOCKED.into())
}

/// The secret, if the keyring has it.
pub fn get(name: &str) -> Result<Option<String>, String> {
    run(async {
        let k = oo7::Keyring::new().await?;
        k.unlock().await?;
        let items = k.search_items(&attributes(name)).await?;
        let Some(item) = items.first() else { return Ok(None) };
        Ok(Some(String::from_utf8_lossy(&item.secret().await?).to_string()))
    })
}

pub fn set(name: &str, label: &str, value: &str) -> Result<(), String> {
    run(async {
        let k = oo7::Keyring::new().await?;
        k.unlock().await?;
        k.create_item(label, &attributes(name), value.as_bytes(), true).await
    })
}

pub fn delete(name: &str) -> Result<(), String> {
    run(async {
        let k = oo7::Keyring::new().await?;
        k.unlock().await?;
        k.delete(&attributes(name)).await
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_keyrings_file_is_named_by_its_path() {
        assert_eq!(keyring_file("/org/freedesktop/secrets/collection/login"), "login");
        assert_eq!(keyring_file("/org/freedesktop/secrets/collection/Default_5fkeyring"), "Default_keyring");
        assert_eq!(keyring_file("/org/freedesktop/secrets/collection/_2e_2e_2fx"), ".._x");
        assert_eq!(keyring_file("/x/a_"), "a_");
    }
}

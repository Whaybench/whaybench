//! Secrets kept in the person's keyring through the Secret Service (spec 3.22): KWallet, GNOME Keyring, oo7 or
//! KeePassXC, whichever runs. Nothing secret is written to a file or a log. A locked keyring can wait on a prompt
//! nobody sees, so every call has a time limit.
use std::{collections::HashMap, time::Duration};

/// The long-lived Claude Code token (`claude setup-token`) the project containers' agents sign in with.
pub const CLAUDE_TOKEN: &str = "claude-token";
/// How the keyring's own tools name it.
pub const CLAUDE_TOKEN_LABEL: &str = "Whaybench: Claude Code token for project containers";

const LIMIT: Duration = Duration::from_secs(15);

fn attributes(name: &str) -> HashMap<&str, &str> { HashMap::from([("application", "whaybench"), ("secret", name)]) }

fn run<T>(work: impl std::future::Future<Output = Result<T, oo7::Error>>) -> Result<T, String> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().map_err(|e| e.to_string())?;
    rt.block_on(async { tokio::time::timeout(LIMIT, work).await })
        .map_err(|_| "the keyring did not answer (locked, waiting for its password?)".to_string())?
        .map_err(|e| format!("keyring: {e}"))
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

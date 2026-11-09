//! Free instructions of a received add-on go into the AI as written, so they are used only after the person read
//! and confirmed them; a confirmation holds for that exact text.
use serde_json::{Value, json};

use crate::{Res, Store, read, s, sha, write_json};

impl Store {
    fn trust_file(&self) -> std::path::PathBuf { self.data.root.join("addon-trust.json") }

    /// Whether instructions from this file may go to the AI without asking.
    pub fn trusted(&self, path: &str, said: &str) -> bool {
        if said.trim().is_empty() { return true; }
        read(&self.trust_file()).is_some_and(|k| s(&k, path) == sha(said))
    }

    pub fn trust(&self, path: &str, said: &str) -> Res<()> {
        let mut k = read(&self.trust_file()).unwrap_or_else(|| json!({}));
        k[path] = Value::String(sha(said));
        write_json(&self.trust_file(), &k)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_confirmation_holds_for_that_text() {
        let st = crate::scratch("trust");
        assert!(st.trusted("/a.json", ""), "nothing to confirm");
        assert!(!st.trusted("/a.json", "do this"));
        st.trust("/a.json", "do this").unwrap();
        assert!(st.trusted("/a.json", "do this"));
        assert!(!st.trusted("/a.json", "do that"), "a changed text asks again");
    }
}

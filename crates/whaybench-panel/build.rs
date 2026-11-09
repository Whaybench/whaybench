//! The commit the panels were built from, for Whaybench → About.
use std::{path::Path, process::Command};

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let commit = Command::new("git").args(["-C", root.to_str().unwrap(), "log", "-1", "--format=%h %cs"]).output().ok()
        .filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()).unwrap_or_default();
    println!("cargo:rustc-env=WB_COMMIT={commit}");
    // A new commit moves the branch head: build again so the stamp follows.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");
}

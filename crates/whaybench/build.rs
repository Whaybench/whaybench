//! Facts only the build knows, for the record's header.
use std::{fs, path::Path, process::Command};

fn output(cmd: &str, args: &[&str]) -> String {
    Command::new(cmd).args(args).output().ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    println!("cargo:rustc-env=WB_COMMIT={}", output("git", &["-C", root.to_str().unwrap(), "log", "-1", "--format=%h %cs"]));
    println!("cargo:rustc-env=WB_RUSTC={}", output("rustc", &["--version"]));
    let rev = fs::read_to_string(root.join("third_party/smithay.rev")).unwrap_or_default();
    println!("cargo:rustc-env=WB_SMITHAY={}", rev.trim());
    println!("cargo:rerun-if-changed=../../third_party/smithay.rev");
    println!("cargo:rerun-if-changed=../../releases");
    // A new commit moves the branch head: build again so the stamp follows.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/refs/heads");

    // The newest release record, built into the program. No record yet means nothing to compare with.
    let mut newest: Option<((u32, u32, u32), String)> = None;
    if let Ok(dir) = fs::read_dir(root.join("releases")) {
        for f in dir.flatten() {
            let name = f.file_name().to_string_lossy().to_string();
            let Some(v) = name.strip_suffix(".toml") else { continue };
            let n: Vec<u32> = v.split('.').filter_map(|x| x.parse().ok()).collect();
            if n.len() == 3 && newest.as_ref().is_none_or(|(b, _)| (n[0], n[1], n[2]) > *b) {
                newest = Some(((n[0], n[1], n[2]), fs::read_to_string(f.path()).unwrap_or_default()));
            }
        }
    }
    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("release.toml");
    fs::write(out, newest.map(|n| n.1).unwrap_or_default()).unwrap();
}

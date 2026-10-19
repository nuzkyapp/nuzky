use std::process::Command;

fn main() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let commit = git(&["rev-parse", "--short=7", "HEAD"])
        .map(|mut commit| {
            if git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty()) {
                commit.push_str("-dirty");
            }
            commit
        })
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=NUZKY_COMMIT={commit}");
    let reference = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    for name in [Some("HEAD"), reference.as_deref(), Some("index")].into_iter().flatten() {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    tauri_build::build()
}

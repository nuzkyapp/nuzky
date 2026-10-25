use std::process::Command;

include!("src/commands.rs");

fn main() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|out| out.status.success())
            .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let commit = git(&["rev-parse", "--short=7", "HEAD"]).unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=NUZKY_COMMIT={commit}");
    let reference = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    for name in [Some("HEAD"), reference.as_deref()].into_iter().flatten() {
        if let Some(path) = git(&["rev-parse", "--git-path", name]) {
            println!("cargo:rerun-if-changed={path}");
        }
    }
    // whisper.cpp links the Vulkan loader, which Windows has only with a graphics driver. Loading it on first use
    // lets Nuzky start without one; crates/analysis/build.rs says why speech then runs on the processor.
    if std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc") {
        println!("cargo:rustc-link-arg=/DELAYLOAD:vulkan-1.dll");
        println!("cargo:rustc-link-arg=delayimp.lib");
    }
    println!("cargo:rerun-if-changed=src/commands.rs");
    // Without an app manifest Tauri lets every local webview call every app command. The permission is written here
    // and not through `AppManifest::commands`, which writes a file per command into src-tauri/permissions; left
    // there, they would give an older checkout an app manifest without the permission and refuse its every command.
    let permissions =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR")).join("app-acl");
    std::fs::create_dir_all(&permissions).expect("creating the app permissions folder");
    let commands: Vec<String> = COMMANDS.iter().map(|command| format!("{command:?}")).collect();
    let permission = format!(
        "[[permission]]\nidentifier = \"main-ui\"\ndescription = \"Every app command, for the editor's own page.\"\n\
         commands.allow = [{}]\n",
        commands.join(", ")
    );
    std::fs::write(permissions.join("main-ui.toml"), permission).expect("writing the main-ui permission");
    let pattern = permissions.join("*.toml").to_string_lossy().into_owned().leak();
    let manifest = tauri_build::AppManifest::new().permissions_path_pattern(pattern);
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(manifest))
        .unwrap_or_else(|error| panic!("{error:#}"))
}

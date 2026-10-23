//! A fake program of the UI flows on Windows, which runs no script by its first line. Copied as `<name>.exe`, it runs
//! the Python script that `NUZKY_FAKE_<NAME>` names with its own arguments, and tells it in `NUZKY_FAKE_AS` the path
//! of the program it plays, which a script on Unix finds in its own `argv[0]` or `__file__`. tests/e2e/harness.py
//! builds it with `rustc`, which puts the Python of the run in `NUZKY_PYTHON`.
use std::process::{Command, exit};

fn main() {
    let exe = std::env::current_exe().expect("the launcher's own path");
    let name = exe.file_stem().and_then(|stem| stem.to_str()).expect("the launcher's own name").to_owned();
    let key = format!("NUZKY_FAKE_{}", name.to_uppercase().replace('-', "_"));
    let Some(script) = std::env::var_os(&key) else {
        eprintln!("{key} does not name the script {name}.exe stands for");
        exit(1);
    };
    // An absolute Python, since a program may be started with a PATH of its own.
    let status = Command::new(env!("NUZKY_PYTHON"))
        .args(["-X", "utf8"])
        .arg(&script)
        .args(std::env::args_os().skip(1))
        .env("NUZKY_FAKE_AS", &exe)
        .status();
    exit(match status {
        Ok(status) => status.code().unwrap_or(1),
        Err(error) => {
            eprintln!("{}: {error}", script.to_string_lossy());
            1
        }
    });
}

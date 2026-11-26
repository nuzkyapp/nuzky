//! On Linux and Windows whisper.cpp and ggml are shared libraries, and ggml's processor code is built once per
//! instruction set, from SSE4.2 to AVX-512. ggml loads the fastest variant this processor runs, so speech recognition
//! works on every x86-64 processor and is as fast as before on those with AVX2. The Vulkan backend is a library of its
//! own too, which does not load without a Vulkan driver; speech then runs on the processor. macOS links them in.
use anyhow::Result;

static ON_ABORT: std::sync::OnceLock<fn(&str)> = std::sync::OnceLock::new();

/// Gives `hook` why ggml stops the app, just before it aborts, as a failed `GGML_ASSERT` in its Vulkan backend does.
pub fn on_abort(hook: fn(&str)) {
    ON_ABORT.set(hook).ok();
}

unsafe extern "C" fn aborting(message: *const std::ffi::c_char) {
    if let (Some(hook), false) = (ON_ABORT.get(), message.is_null()) {
        // SAFETY: ggml passes its message as a C string on its stack.
        hook(&unsafe { std::ffi::CStr::from_ptr(message) }.to_string_lossy());
    }
}

/// Loads ggml's backends once, before anything calls into whisper.cpp; every later call answers from that attempt.
pub(crate) fn ready() -> Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    {
        static LOADED: std::sync::OnceLock<Result<(), String>> = std::sync::OnceLock::new();
        if let Err(reason) = LOADED.get_or_init(load) {
            return Err(anyhow::anyhow!("Speech recognition cannot start: {reason}"));
        }
    }
    static WATCHED: std::sync::Once = std::sync::Once::new();
    // SAFETY: ggml keeps the function, which lives as long as the app.
    WATCHED.call_once(|| unsafe {
        whisper_rs::whisper_rs_sys::ggml_set_abort_callback(Some(aborting));
    });
    Ok(())
}

#[cfg(any(target_os = "linux", windows))]
fn load() -> Result<(), String> {
    use std::path::PathBuf;
    use whisper_rs::whisper_rs_sys as sys;

    let exe = std::env::current_exe().and_then(dunce::canonicalize).map_err(|e| format!("finding Nuzky: {e}"))?;
    let dir = exe.parent().unwrap_or(std::path::Path::new("."));
    // Tauri's resource directory, named after productName in src-tauri/tauri.conf.json, or beside nuzky-app.exe.
    let installed = if cfg!(windows) { dir.to_path_buf() } else { dir.join("../lib/Nuzky") };
    // Only a debug build looks in the build's own directory, as with ONNX Runtime (crates/vision/src/runtime.rs).
    let built = cfg!(debug_assertions).then(|| option_env!("NUZKY_GGML_DIR").map(PathBuf::from)).flatten();
    let candidates: Vec<PathBuf> = std::iter::once(installed).chain(built).collect();
    let cpu = if cfg!(windows) { "ggml-cpu" } else { "libggml-cpu" };
    let has_cpu = |dir: &PathBuf| {
        std::fs::read_dir(dir).is_ok_and(|mut files| {
            files.any(|file| file.is_ok_and(|file| file.file_name().to_string_lossy().starts_with(cpu)))
        })
    };
    let Some(dir) = candidates.iter().find(|dir| has_cpu(dir)) else {
        return Err(format!("its libraries are missing ({}); reinstall Nuzky", candidates[0].display()));
    };
    #[cfg(windows)]
    preload(dir)?;
    let path = dir.to_str().and_then(|dir| std::ffi::CString::new(dir).ok()).ok_or("its folder has an unusual name")?;
    // SAFETY: ggml copies the path; it registers each backend it can load and skips the others.
    unsafe { sys::ggml_backend_load_all_from_path(path.as_ptr()) };
    // SAFETY: only reads the device list the call above built.
    if unsafe { sys::ggml_backend_dev_by_type(sys::ggml_backend_dev_type_GGML_BACKEND_DEVICE_TYPE_CPU) }.is_null() {
        return Err(format!("none of its processor libraries in {} runs on this processor", dir.display()));
    }
    Ok(())
}

/// The binaries load whisper.cpp's libraries only when first called (`/DELAYLOAD`, crates/analysis/link_ggml.rs), by
/// name. Loaded here first by their full path, they are the ones those calls get.
#[cfg(windows)]
fn preload(dir: &std::path::Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn LoadLibraryExW(name: *const u16, file: *mut std::ffi::c_void, flags: u32) -> *mut std::ffi::c_void;
    }
    const LOAD_WITH_ALTERED_SEARCH_PATH: u32 = 0x8;
    for library in ["ggml-base.dll", "ggml.dll", "whisper.dll"] {
        let path: Vec<u16> = dir.join(library).as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: path is NUL-terminated; the library stays loaded for the process.
        if unsafe { LoadLibraryExW(path.as_ptr(), std::ptr::null_mut(), LOAD_WITH_ALTERED_SEARCH_PATH) }.is_null() {
            return Err(format!("{library} did not load ({}); reinstall Nuzky", std::io::Error::last_os_error()));
        }
    }
    Ok(())
}

/// Fails unless whisper.cpp knows the language code; "auto" detects it.
pub fn check_language(code: &str) -> Result<()> {
    if code == "auto" {
        return Ok(());
    }
    ready()?;
    anyhow::ensure!(!code.contains('\0') && whisper_rs::get_lang_id(code).is_some(), "Unknown language: {code}");
    Ok(())
}

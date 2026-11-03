//! Full access for a plugin whose manifest asks for it: `nuzky.fs`, `nuzky.fetch` and `nuzky.exec` in its script,
//! which reach the user's files, the internet and other programs as the user could. They work only in a run the user
//! started from the app after allowing this version of the plugin; in a preview, or when Nuzky checks the script on
//! saving, they throw, so changing a setting never writes, sends or starts anything. The user's consent belongs to
//! Nuzky's data, outside the plugins folder, so neither the plugin nor an agent's `save_plugin` can give it.
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, UNIX_EPOCH};

use anyhow::{Context, Result, ensure};
use rquickjs::{ArrayBuffer, Ctx, Exception, Function, Object, TypedArray, Value as JsValue, prelude::Opt};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

/// What the script's `nuzky.fs`, `nuzky.fetch` and `nuzky.exec` do.
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Reach {
    /// A plugin without full access has none of them.
    None,
    /// They throw: a preview, or the check when the plugin is saved.
    Preview,
    Live,
}

/// The longest a response or a file read at once may be; QuickJS holds at most 256 MB anyway.
const MAX_READ: u64 = 128 << 20;

/// Which version of the plugin the user allowed: its manifest and script, so any change asks again.
pub(crate) fn key(manifest: &str, script: &str) -> String {
    let mut hash = Sha256::new();
    for part in ["plugin.json", manifest, "plugin.js", script] {
        hash.update((part.len() as u64).to_le_bytes());
        hash.update(part);
    }
    hash.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn store() -> Result<PathBuf> {
    Ok(nuzky_engine::data_dir().context("STORE_UNAVAILABLE: no data directory")?.join("plugin-access.json"))
}

fn read_store() -> BTreeMap<String, String> {
    store()
        .ok()
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn write_store(allowed: &BTreeMap<String, String>) -> Result<()> {
    let path = store()?;
    let json = serde_json::to_vec_pretty(allowed)?;
    crate::sounds::publish(&path, |out| Ok(out.write_all(&json)?))
        .context("STORE_UNWRITABLE: cannot save the plugin's access")
}

pub(crate) fn allowed(id: &str, key: &str) -> bool {
    read_store().get(id).is_some_and(|allowed| allowed == key)
}

/// Records that the user allowed plugin `id` as it is now, which `key` must still name.
pub fn allow(id: &str, key: &str) -> Result<()> {
    let plugin =
        crate::plugins::list()?.into_iter().find(|p| p.id == id).context("PLUGIN_NOT_FOUND: the plugin is gone")?;
    ensure!(
        plugin.access_key.as_deref() == Some(key),
        "PLUGIN_CHANGED: The plugin changed since you looked at it. Check it again, then allow it."
    );
    let mut allowed = read_store();
    allowed.insert(id.to_owned(), key.to_owned());
    write_store(&allowed)
}

pub fn forget(id: &str) -> Result<()> {
    let mut allowed = read_store();
    if allowed.remove(id).is_some() {
        write_store(&allowed)?;
    }
    Ok(())
}

/// Where a plugin with full access finds things: the project, its media by asset id, the plugin's own folder and the
/// user's usual folders.
pub(crate) fn paths(project: &Path, plugin: &Path, media: &nuzky_engine::Project) -> Value {
    let text =
        |path: Option<PathBuf>| path.map(|p| Value::String(p.to_string_lossy().into_owned())).unwrap_or(Value::Null);
    let media: Map<String, Value> =
        media.assets.iter().map(|asset| (asset.id.clone(), Value::String(asset.path.clone()))).collect();
    serde_json::json!({
        "project": text(Some(project.to_path_buf())),
        "projectDir": text(project.parent().map(Path::to_path_buf)),
        "media": media,
        "plugin": text(Some(plugin.to_path_buf())),
        "home": text(dirs::home_dir()),
        "documents": text(dirs::document_dir()),
        "downloads": text(dirs::download_dir()),
        "desktop": text(dirs::desktop_dir()),
        "videos": text(dirs::video_dir()),
        "temp": text(Some(std::env::temp_dir())),
    })
}

fn fail<T>(ctx: &Ctx, what: &str, error: impl std::fmt::Display) -> rquickjs::Result<T> {
    Err(Exception::throw_message(ctx, &format!("{what}: {error}")))
}

/// A string, a Uint8Array or an ArrayBuffer as bytes.
fn bytes<'js>(ctx: &Ctx<'js>, what: &str, data: &JsValue<'js>) -> rquickjs::Result<Vec<u8>> {
    if let Some(text) = data.as_string() {
        return Ok(text.to_string()?.into_bytes());
    }
    if let Ok(array) = TypedArray::<u8>::from_value(data.clone()) {
        // SAFETY: copied before any JavaScript runs again.
        return Ok(unsafe { array.as_bytes() }.unwrap_or_default().to_vec());
    }
    if let Some(buffer) = data.as_object().and_then(|o| ArrayBuffer::from_object(o.clone())) {
        // SAFETY: as above.
        return Ok(unsafe { buffer.as_bytes() }.unwrap_or_default().to_vec());
    }
    fail(ctx, what, "give a string, a Uint8Array or an ArrayBuffer")
}

fn read_limited(mut reader: impl Read, what: &str) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    (&mut reader).take(MAX_READ + 1).read_to_end(&mut out)?;
    ensure!(out.len() as u64 <= MAX_READ, "{what} is larger than {} MB", MAX_READ >> 20);
    Ok(out)
}

const FS: [&str; 10] = ["read", "readBytes", "write", "append", "list", "stat", "mkdir", "remove", "rename", "copy"];

/// In a preview each function throws, whatever it is given, before it does anything.
fn refuse_all<'js>(ctx: &Ctx<'js>, nuzky: &Object<'js>) -> rquickjs::Result<()> {
    let refusal = |name: String| {
        move |ctx: Ctx<'js>, _: rquickjs::prelude::Rest<JsValue<'js>>| -> rquickjs::Result<()> {
            fail(
                &ctx,
                "PLUGIN_PREVIEW",
                format!("{name} does not run in a preview; return before it when ctx.preview is true"),
            )
        }
    };
    let fs = Object::new(ctx.clone())?;
    for name in FS {
        fs.set(name, Function::new(ctx.clone(), refusal(format!("nuzky.fs.{name}")))?)?;
    }
    nuzky.set("fs", fs)?;
    for name in ["fetch", "exec"] {
        nuzky.set(name, Function::new(ctx.clone(), refusal(format!("nuzky.{name}")))?)?;
    }
    Ok(())
}

/// Adds `nuzky.fs`, `nuzky.fetch` and `nuzky.exec` to `nuzky` as `reach` allows.
pub(crate) fn install<'js>(ctx: &Ctx<'js>, nuzky: &Object<'js>, reach: Reach) -> rquickjs::Result<()> {
    if reach == Reach::None {
        return Ok(());
    }
    if reach == Reach::Preview {
        return refuse_all(ctx, nuzky);
    }
    let fs = Object::new(ctx.clone())?;
    fs.set(
        "read",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<String> {
            let bytes = std::fs::File::open(&path).map_err(anyhow::Error::from).and_then(|f| read_limited(f, &path));
            match bytes.map(String::from_utf8) {
                Ok(Ok(text)) => Ok(text),
                Ok(Err(_)) => fail(&ctx, "nuzky.fs.read", format!("{path} is not UTF-8 text; use nuzky.fs.readBytes")),
                Err(error) => fail(&ctx, "nuzky.fs.read", format!("{path}: {error:#}")),
            }
        })?,
    )?;
    fs.set(
        "readBytes",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<TypedArray<'js, u8>> {
            match std::fs::File::open(&path).map_err(anyhow::Error::from).and_then(|f| read_limited(f, &path)) {
                Ok(bytes) => TypedArray::new(ctx, bytes),
                Err(error) => fail(&ctx, "nuzky.fs.readBytes", format!("{path}: {error:#}")),
            }
        })?,
    )?;
    for (name, append) in [("write", false), ("append", true)] {
        fs.set(
            name,
            Function::new(
                ctx.clone(),
                move |ctx: Ctx<'js>, path: String, data: JsValue<'js>| -> rquickjs::Result<()> {
                    let what = if append { "nuzky.fs.append" } else { "nuzky.fs.write" };
                    let data = bytes(&ctx, what, &data)?;
                    let written = std::fs::OpenOptions::new()
                        .create(true)
                        .write(true)
                        .append(append)
                        .truncate(!append)
                        .open(&path)
                        .and_then(|mut file| file.write_all(&data));
                    written.or_else(|error| fail(&ctx, what, format!("{path}: {error}")))
                },
            )?,
        )?;
    }
    fs.set(
        "list",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<rquickjs::Array<'js>> {
            let entries = match std::fs::read_dir(&path) {
                Ok(entries) => entries,
                Err(error) => return fail(&ctx, "nuzky.fs.list", format!("{path}: {error}")),
            };
            let list = rquickjs::Array::new(ctx.clone())?;
            let mut rows: Vec<_> = entries.filter_map(Result::ok).collect();
            rows.sort_by_key(|entry| entry.file_name());
            for (i, entry) in rows.into_iter().enumerate() {
                let row = Object::new(ctx.clone())?;
                let meta = entry.metadata().ok();
                row.set("name", entry.file_name().to_string_lossy().into_owned())?;
                row.set("path", entry.path().to_string_lossy().into_owned())?;
                row.set("isDir", meta.as_ref().is_some_and(|m| m.is_dir()))?;
                row.set("size", meta.as_ref().map_or(0.0, |m| m.len() as f64))?;
                list.set(i, row)?;
            }
            Ok(list)
        })?,
    )?;
    fs.set(
        "stat",
        Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: String| -> rquickjs::Result<JsValue<'js>> {
            let Ok(meta) = std::fs::metadata(&path) else { return Ok(JsValue::new_null(ctx)) };
            let row = Object::new(ctx.clone())?;
            row.set("isDir", meta.is_dir())?;
            row.set("isFile", meta.is_file())?;
            row.set("size", meta.len() as f64)?;
            let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok());
            row.set("modifiedMs", modified.map_or(0.0, |d| d.as_millis() as f64))?;
            Ok(row.into_value())
        })?,
    )?;
    type Op = fn(&str, &str) -> std::io::Result<()>;
    let ops: [(&str, &'static str, Op); 4] = [
        ("mkdir", "nuzky.fs.mkdir", |path, _| std::fs::create_dir_all(path)),
        ("remove", "nuzky.fs.remove", |path, _| match std::fs::symlink_metadata(path) {
            Ok(meta) if meta.is_dir() => std::fs::remove_dir_all(path),
            _ => std::fs::remove_file(path),
        }),
        ("rename", "nuzky.fs.rename", |from, to| std::fs::rename(from, to)),
        ("copy", "nuzky.fs.copy", |from, to| std::fs::copy(from, to).map(drop)),
    ];
    for (name, what, op) in ops {
        fs.set(
            name,
            Function::new(ctx.clone(), move |ctx: Ctx<'js>, path: String, to: Opt<String>| -> rquickjs::Result<()> {
                let two = matches!(name, "rename" | "copy");
                if two && to.0.is_none() {
                    return fail(&ctx, what, "give the path it goes to");
                }
                let to = to.0.unwrap_or_default();
                op(&path, &to).or_else(|error| fail(&ctx, what, format!("{path}: {error}")))
            })?,
        )?;
    }
    nuzky.set("fs", fs)?;

    nuzky.set(
        "fetch",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>, url: String, options: Opt<Object<'js>>| -> rquickjs::Result<Object<'js>> {
                fetch(&ctx, &url, options.0)
            },
        )?,
    )?;
    nuzky.set(
        "exec",
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'js>,
                  program: String,
                  args: Opt<Vec<String>>,
                  options: Opt<Object<'js>>|
                  -> rquickjs::Result<Object<'js>> {
                exec(&ctx, &program, args.0.unwrap_or_default(), options.0)
            },
        )?,
    )?;
    Ok(())
}

fn fetch<'js>(ctx: &Ctx<'js>, url: &str, options: Option<Object<'js>>) -> rquickjs::Result<Object<'js>> {
    let get = |key: &str| {
        options.as_ref().and_then(|o| o.get::<_, JsValue>(key).ok()).filter(|v| !v.is_undefined() && !v.is_null())
    };
    let method = match get("method") {
        Some(method) => method.as_string().map(|s| s.to_string()).transpose()?.unwrap_or_default().to_uppercase(),
        None => "GET".into(),
    };
    let binary = get("binary").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut request = ureq::http::Request::builder().method(method.as_str()).uri(url);
    if let Some(headers) = get("headers").and_then(|v| v.into_object()) {
        for name in headers.keys::<String>() {
            let name = name?;
            let value: String = headers.get(&name)?;
            request = request.header(name, value);
        }
    }
    let body = match get("body") {
        Some(body) => bytes(ctx, "nuzky.fetch", &body)?,
        None => Vec::new(),
    };
    let request = match request.body(body) {
        Ok(request) => request,
        Err(error) => return fail(ctx, "nuzky.fetch", format!("{url}: {error}")),
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .allow_non_standard_methods(true)
        .timeout_connect(Some(Duration::from_secs(30)))
        .build()
        .into();
    let response = match agent.run(request) {
        Ok(response) => response,
        Err(error) => return fail(ctx, "nuzky.fetch", format!("{url}: {error}")),
    };
    let out = Object::new(ctx.clone())?;
    let status = response.status().as_u16();
    out.set("status", status)?;
    out.set("ok", (200..300).contains(&status))?;
    let headers = Object::new(ctx.clone())?;
    for (name, value) in response.headers() {
        headers.set(name.as_str(), value.to_str().unwrap_or_default())?;
    }
    out.set("headers", headers)?;
    let body = match read_limited(response.into_body().into_reader(), "the response") {
        Ok(body) => body,
        Err(error) => return fail(ctx, "nuzky.fetch", format!("{url}: {error:#}")),
    };
    if binary {
        out.set("body", TypedArray::<u8>::new(ctx.clone(), body)?)?;
    } else {
        out.set("body", String::from_utf8_lossy(&body).into_owned())?;
    }
    Ok(out)
}

fn exec<'js>(
    ctx: &Ctx<'js>,
    program: &str,
    args: Vec<String>,
    options: Option<Object<'js>>,
) -> rquickjs::Result<Object<'js>> {
    let get = |key: &str| {
        options.as_ref().and_then(|o| o.get::<_, JsValue>(key).ok()).filter(|v| !v.is_undefined() && !v.is_null())
    };
    let mut command = Command::new(program);
    command.args(&args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped());
    if let Some(cwd) = get("cwd").and_then(|v| v.as_string().cloned()) {
        command.current_dir(cwd.to_string()?);
    }
    if let Some(env) = get("env").and_then(|v| v.into_object()) {
        for name in env.keys::<String>() {
            let name = name?;
            let value: String = env.get(&name)?;
            command.env(name, value);
        }
    }
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000); // CREATE_NO_WINDOW
    let input = match get("input") {
        Some(input) => bytes(ctx, "nuzky.exec", &input)?,
        None => Vec::new(),
    };
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return fail(ctx, "nuzky.exec", format!("{program}: {error}")),
    };
    let mut stdin = child.stdin.take();
    // Written beside the reading, so a program that answers before it has read everything never waits on us.
    let writer = std::thread::spawn(move || {
        if let Some(stdin) = stdin.as_mut() {
            stdin.write_all(&input).ok();
        }
    });
    let output = child.wait_with_output();
    writer.join().ok();
    let output = match output {
        Ok(output) => output,
        Err(error) => return fail(ctx, "nuzky.exec", format!("{program}: {error}")),
    };
    let out = Object::new(ctx.clone())?;
    out.set("code", output.status.code())?;
    out.set("stdout", String::from_utf8_lossy(&output.stdout).into_owned())?;
    out.set("stderr", String::from_utf8_lossy(&output.stderr).into_owned())?;
    Ok(out)
}

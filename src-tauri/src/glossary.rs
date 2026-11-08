use nuzky_mcp::glossary;

use crate::{CmdResult, err};

#[tauri::command]
pub fn saved_names() -> CmdResult<Vec<String>> {
    glossary::list().map_err(err)
}

#[tauri::command]
pub fn save_name(name: String) -> CmdResult<Vec<String>> {
    glossary::add(&name).map_err(err)
}

#[tauri::command]
pub fn rename_name(from: String, to: String) -> CmdResult<Vec<String>> {
    glossary::rename(&from, &to).map_err(err)
}

#[tauri::command]
pub fn delete_name(name: String) -> CmdResult<Vec<String>> {
    glossary::remove(&name).map_err(err)
}

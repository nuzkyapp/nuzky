import { join, videoDir } from "@tauri-apps/api/path";
import { save, type DialogFilter } from "@tauri-apps/plugin-dialog";
import { useEditor } from "./store";

const KEY = "nuzky.exportFolders";

/** The folder each project last exported to, by its file; "" holds the one any project last exported to. */
function folders(): Record<string, string> {
  try {
    return (JSON.parse(localStorage.getItem(KEY) ?? "{}") as Record<string, string> | null) ?? {};
  } catch {
    return {};
  }
}

async function exportPath(name: string) {
  const saved = folders();
  const folder = saved[useEditor.getState().snap?.path ?? ""] ?? saved[""];
  try {
    return await join(folder ?? (await videoDir()), name);
  } catch {
    return name; // no Videos folder; the dialog falls back to its default location
  }
}

/** The save dialogs a WebDriver flow answered, by the path each offered; see `answerNextSave`. */
export const savesAsked: string[] = [];
let answer: string | true | null = null;

/**
 * Makes the next save dialog of an export return `path`, or the path it offers as Save does, for a WebDriver flow in a
 * development build: on macOS the dialog runs in another process, which the flow cannot press.
 */
export function answerNextSave(path?: string) {
  answer = path ?? true;
}

/**
 * Asks where to save an export, named `name`. Every export, the video, its captions, the cover and the recap, starts
 * in the folder this project last exported to, else the one any project did, else Videos, so they land together;
 * the folder chosen is remembered. Null when cancelled.
 */
export async function askExportPath(name: string, filters: DialogFilter[]) {
  const defaultPath = await exportPath(name);
  const answered = import.meta.env.DEV ? answer : null;
  answer = null;
  if (answered) savesAsked.push(defaultPath);
  const picked = answered ? (answered === true ? defaultPath : answered) : await save({ defaultPath, filters });
  const folder = picked?.replace(/[\\/][^\\/]*$/, "");
  if (picked && folder && folder !== picked) {
    localStorage.setItem(KEY, JSON.stringify({ ...folders(), [useEditor.getState().snap?.path ?? ""]: folder, "": folder }));
  }
  return picked;
}

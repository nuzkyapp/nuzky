import { create } from "zustand";
import { api, errorText, plainError } from "./api";
import { useEditor } from "./store";

/** Names the user saved for speech recognition, oldest first; null until read. */
export const useNames = create<{ names: string[] | null; error: string | null }>(() => ({ names: null, error: null }));

/** Each must match the engine's glossary.rs. */
export const MAX_NAMES = 40;
export const MAX_NAME_CHARS = 40;

const sameName = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A corrected word as a name: without the sentence's punctuation around it, "Codex," → "Codex". */
export const nameOf = (word: string) => word.trim().replace(/^[\s"'“”„‚‘’«»(\[]+|[\s"'“”„‚‘’«».,!?;:…)\]]+$/gu, "");

/** True when `name` is saved already, cased as it is or not. */
export const isSaved = (name: string) => (useNames.getState().names ?? []).some((n) => sameName(n, name));

async function change(run: () => Promise<string[]>): Promise<boolean> {
  try {
    useNames.setState({ names: await run(), error: null });
    return true;
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) });
    return false;
  }
}

export async function loadNames() {
  try {
    useNames.setState({ names: await api.savedNames(), error: null });
  } catch (e) {
    useNames.setState({ names: [], error: plainError(errorText(e)) });
  }
}

export const saveName = (name: string) => change(() => api.saveName(name));
export const renameName = (from: string, to: string) => change(() => api.renameName(from, to));
export const deleteName = (name: string) => change(() => api.deleteName(name));

/** Saves a corrected word the user chose to keep, and says what that does, with Undo. */
export async function rememberName(name: string) {
  if (!(await saveName(name))) return;
  // The newest name, as the backend wrote it: with single spaces and composed accents.
  const saved = useNames.getState().names?.at(-1) ?? name;
  useEditor.getState().toast({
    kind: "success",
    text: `Saved “${saved}”. Transcripts made from now on are likelier to spell it this way.`,
    action: { label: "Undo", run: () => void deleteName(saved) },
  });
}

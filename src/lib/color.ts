import { useEffect, useState } from "react";
import { create } from "zustand";
import { open as pickFile } from "@tauri-apps/plugin-dialog";
import { api, errorText, plainError } from "./api";
import { findClip, useEditor } from "./store";
import type { Clip, LogProfile, Lut } from "./types";

/** A LUT of the user's library (src-tauri/src/luts.rs). */
export interface LutInfo {
  id: string;
  name: string;
}

/** A look a Filters tile shows: a log profile, then a LUT of the library at a strength. */
export interface ColorLook {
  log: LogProfile;
  lut: string | null;
  strength: number;
}

type Profile = { id: Exclude<LogProfile, "none">; label: string; name: string; recordedOn?: string };

/** The camera log profiles Nuzky converts (crates/engine/src/color.rs), grouped as the Filters tab lists them. */
export const LOG_GROUPS: { name: string; profiles: Profile[] }[] = [
  {
    name: "Phones",
    profiles: [
      { id: "appleLog", label: "Apple Log", name: "Apple Log", recordedOn: "iPhone 15 Pro and 16 Pro" },
      { id: "appleLog2", label: "Apple Log 2", name: "Apple Log 2", recordedOn: "iPhone 17 Pro and later" },
      { id: "xiaomiLog", label: "Xiaomi Log", name: "Xiaomi Log" },
    ],
  },
  {
    name: "Sony",
    profiles: [
      { id: "sLog3Cine", label: "S-Log3 Cine", name: "Sony S-Log3 · S-Gamut3.Cine" },
      { id: "sLog3", label: "S-Log3", name: "Sony S-Log3 · S-Gamut3" },
      { id: "sLog2", label: "S-Log2", name: "Sony S-Log2" },
    ],
  },
  {
    name: "Canon",
    profiles: [
      { id: "canonLog3", label: "Canon Log 3", name: "Canon Log 3" },
      { id: "canonLog2", label: "Canon Log 2", name: "Canon Log 2" },
    ],
  },
  {
    name: "Panasonic, Fujifilm, Nikon, Leica",
    profiles: [
      { id: "vLog", label: "V-Log", name: "Panasonic V-Log" },
      { id: "fLog", label: "F-Log", name: "Fujifilm F-Log" },
      { id: "fLog2", label: "F-Log2", name: "Fujifilm F-Log2" },
      { id: "nLog", label: "N-Log", name: "Nikon N-Log" },
      { id: "lLog", label: "L-Log", name: "Leica L-Log" },
    ],
  },
  {
    name: "DJI, Blackmagic, ARRI",
    profiles: [
      { id: "dLog", label: "D-Log", name: "DJI D-Log" },
      { id: "blackmagicFilm5", label: "Blackmagic", name: "Blackmagic Film Gen 5" },
      { id: "logC3", label: "LogC3", name: "ARRI LogC3" },
      { id: "logC4", label: "LogC4", name: "ARRI LogC4" },
    ],
  },
];

export const LOG_PROFILES = LOG_GROUPS.flatMap((g) => g.profiles);

/** The profiles Log footage shows until Show all: the phones and the cameras creators use most. */
export const COMMON_PROFILES: LogProfile[] = ["appleLog", "appleLog2", "xiaomiLog", "sLog3Cine", "canonLog3", "vLog"];

export const profileOf = (log: LogProfile) => LOG_PROFILES.find((p) => p.id === log);

export const lutOf = (lut: Lut | undefined) => (lut?.type === "file" ? lut : null);

/** The user's LUT library, read when the Filters tab first needs it and after each import or removal. */
export const useLuts = create<{ list: LutInfo[] | null; error: string | null }>(() => ({ list: null, error: null }));

export async function loadLuts() {
  try {
    useLuts.setState({ list: await api.listLuts() });
  } catch (e) {
    useLuts.setState({ list: [] });
    useEditor.getState().toast({ kind: "error", text: `Couldn't read your LUTs: ${plainError(errorText(e))}` });
  }
}

/** The one video or image clip selected, if any. */
export function pictureClip(): Clip | null {
  const s = useEditor.getState();
  if (!s.snap || s.selection.length !== 1) return null;
  const found = findClip(s.snap.project, s.selection[0]);
  const c = found?.clip.content;
  const asset = c?.type === "media" ? s.snap.project.assets.find((a) => a.id === c.assetId) : undefined;
  return found && c?.type === "media" && asset && asset.kind !== "audio" ? found.clip : null;
}

/** Puts the LUT on the clip, keeping the strength of a LUT it has; one undo step. */
export function applyLut(clipId: string, lut: LutInfo) {
  const s = useEditor.getState();
  const found = s.snap && findClip(s.snap.project, clipId);
  const had = found?.clip.content.type === "media" ? lutOf(found.clip.content.lut) : null;
  return s.edit({ type: "updateClip", clipId, lut: { type: "file", id: lut.id, name: lut.name, strength: had?.strength ?? 1 } });
}

/**
 * Copies each `.cube` file into the library and puts the last one that reads on the selected video or image clip.
 * A file Nuzky can't use leaves the library and the clip as they were and says why in the Filters tab.
 */
export async function importLutPaths(paths: string[]) {
  useLuts.setState({ error: null });
  let added: LutInfo | null = null;
  for (const path of paths) {
    try {
      added = await api.importLut(path);
    } catch (e) {
      const name = path.split(/[\\/]/).pop() ?? path;
      useLuts.setState({ error: `Couldn't import “${name}”. ${plainError(errorText(e))}` });
    }
  }
  await loadLuts();
  const clip = pictureClip();
  if (added && clip) await applyLut(clip.id, added);
}

export async function pickAndImportLut() {
  const picked = await pickFile({ multiple: true, filters: [{ name: "LUT", extensions: ["cube", "CUBE"] }] });
  if (picked && picked.length > 0) await importLutPaths(Array.isArray(picked) ? picked : [picked]);
}

export async function removeLut(lut: LutInfo) {
  try {
    await api.removeLut(lut.id);
    useEditor.getState().toast({ kind: "info", text: `Moved “${lut.name}” to the Trash. Clips with it play without it.` });
  } catch (e) {
    useEditor.getState().toast({ kind: "error", text: `Couldn't remove “${lut.name}”: ${plainError(errorText(e))}` });
  }
  await loadLuts();
}

const lookKey = (l: ColorLook) => `${l.log}:${l.lut ?? ""}:${l.strength}`;
/** Previews made this session, by asset and look; a few hundred small PNGs at most. */
const made = new Map<string, string | null>();

/** The asset's thumbnail in each look, made in one call when a look is new; `undefined` while it is being made. */
export function useColorPreviews(assetId: string | null, looks: ColorLook[]): (string | null | undefined)[] {
  const key = looks.map(lookKey).join("|");
  const [, setVersion] = useState(0);
  useEffect(() => {
    if (!assetId) return;
    const wanted = looks.filter((l) => !made.has(`${assetId}|${lookKey(l)}`));
    if (wanted.length === 0) return;
    if (made.size > 400) made.clear();
    let live = true;
    api
      .colorPreviews(assetId, wanted)
      .then((urls) => {
        wanted.forEach((l, i) => made.set(`${assetId}|${lookKey(l)}`, urls?.[i] ?? null));
        if (live) setVersion((v) => v + 1);
      })
      .catch((e) => {
        console.warn("Filters tile previews failed", e);
        wanted.forEach((l) => made.set(`${assetId}|${lookKey(l)}`, null));
        if (live) setVersion((v) => v + 1);
      });
    return () => {
      live = false;
    };
    // `key` stands for `looks`, which is a new array on every render.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [assetId, key]);
  return looks.map((l) => (assetId ? made.get(`${assetId}|${lookKey(l)}`) : null));
}

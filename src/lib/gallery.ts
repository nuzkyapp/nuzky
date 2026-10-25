import { Channel } from "@tauri-apps/api/core";
import { create } from "zustand";
import { api, errorText } from "./api";
import { loadPlugins, openPlugin, setAuto, usePlugins } from "./plugins";
import { useEditor } from "./store";

/**
 * What installing a gallery plugin would do here: `update`, the installed copy is unchanged and the gallery has
 * another; `changed`, the user or their AI changed the installed copy; `clash`, a plugin of the user's own has its id.
 */
export type GalleryState = "available" | "installed" | "update" | "changed" | "clash" | "newer" | "refused";

export interface GalleryPlugin {
  id: string;
  /** Installing sends it back, so only the entry the user was shown installs. */
  key: string;
  name: string;
  /** The author's own words. */
  description: string;
  version: string;
  author: string;
  /** What it reads and does, in Nuzky's words from its manifest; `new` when the installed version could not do it. */
  summary: { text: string; new: boolean }[];
  /** `refused`: the file is why Nuzky won't install it. */
  files: { path: string; size: number; refused: boolean }[];
  size: number;
  state: GalleryState;
  installedVersion: string | null;
}

export const REFUSED = "Nuzky won’t install this plugin. It includes files plugins aren’t allowed to have.";

interface Gallery {
  tab: "installed" | "gallery";
  /** null while the list loads. */
  list: GalleryPlugin[] | null;
  /** Why the list did not load, with its code. */
  error: string | null;
  /** The plugin shown in the inspector. */
  open: string | null;
  installing: { id: string; percent: number } | null;
  /** Why the last install failed, with its code, for the plugin it was. */
  failed: { id: string; text: string } | null;
}

export const useGallery = create<Gallery>(() => ({ tab: "installed", list: null, error: null, open: null, installing: null, failed: null }));

let loads = 0;
let loading: Promise<void> = Promise.resolve();

/** Reads the gallery's list, only while the Gallery is open; `refresh: false` shows the list read last again. */
export function loadGallery(refresh = true) {
  const mine = ++loads;
  if (refresh) useGallery.setState({ list: null, error: null });
  loading = (async () => {
    try {
      const list = await api.galleryPlugins(refresh);
      if (mine === loads) useGallery.setState({ list, error: null });
    } catch (e) {
      if (mine === loads) useGallery.setState({ list: null, error: errorText(e) });
    }
  })();
  return loading;
}

export function showTab(tab: Gallery["tab"]) {
  useGallery.setState({ tab, open: null, failed: null });
  if (tab === "gallery") usePlugins.setState({ open: null, preview: null });
}

export function openGalleryPlugin(id: string | null) {
  useGallery.setState({ open: id, failed: null });
  if (id) usePlugins.setState({ open: null, preview: null });
}

// One plugin in the inspector at a time. An installed one opened from anywhere, such as the launcher, shows its row
// on Installed; a gallery plugin shows only while the Plugins tab is open.
usePlugins.subscribe((s, prev) => {
  if (s.open && s.open !== prev.open) useGallery.setState({ open: null, tab: "installed" });
});
useEditor.subscribe((s) => {
  if (useGallery.getState().open && s.panelTab !== "plugins") useGallery.setState({ open: null });
});

/** Installs or replaces it; `replacing` is the state the user agreed to replace, whose version goes to the Trash. */
export async function installGalleryPlugin(p: GalleryPlugin, replacing: GalleryState | null) {
  if (useGallery.getState().installing) return;
  useGallery.setState({ installing: { id: p.id, percent: 0 }, failed: null });
  const progress = new Channel<number>();
  progress.onmessage = (percent) => useGallery.setState((s) => (s.installing?.id === p.id ? { installing: { id: p.id, percent } } : {}));
  const listedAt = loads;
  // Code the user hasn't seen run must not run by itself, so its automatic runs wait until they turn them on again.
  // They go off before any file changes, so a pass of automatic runs already going on skips the plugin.
  const { auto } = usePlugins.getState();
  const was = (["import", "export"] as const).filter((trigger) => auto[trigger].includes(p.id));
  for (const trigger of was) setAuto(p.id, trigger, false);
  try {
    const list = await api.installGalleryPlugin(p.id, p.key, replacing, progress);
    // A list read meanwhile is newer than the one this install answers with.
    if (listedAt === loads) useGallery.setState({ list, installing: null });
    else {
      useGallery.setState({ installing: null });
      void loading.then(() => useGallery.getState().list && loadGallery(false));
    }
    // Turned on again while it installed, it would run the new code.
    setAuto(p.id, "import", false);
    setAuto(p.id, "export", false);
    // Open is offered while the plugin is in the list, so the list must know it first.
    await loadPlugins();
    // Opening it would run its preview, so only the user's Open does.
    useEditor.getState().toast({
      kind: "success",
      text: `Installed “${p.name}”.`,
      action: { label: "Open", run: () => openPlugin(p.id), valid: () => usePlugins.getState().list?.some((q) => q.id === p.id) ?? true },
    });
  } catch (e) {
    // Nothing was installed, so the version the user turned them on for is still the one that runs.
    const { auto } = usePlugins.getState();
    for (const trigger of was) if (!auto[trigger].includes(p.id)) setAuto(p.id, trigger, true);
    const text = errorText(e);
    // Installed meanwhile, as the user wanted: its state says so.
    const quiet = text.startsWith("CANCELLED") || text.startsWith("GALLERY_INSTALLED");
    useGallery.setState({ installing: null, failed: quiet ? null : { id: p.id, text } });
    // What changed on this computer, or in the list read meanwhile, shows in its state.
    if (text.startsWith("GALLERY_CHANGED") || text.startsWith("GALLERY_INSTALLED") || text.startsWith("GALLERY_STALE")) void loadGallery(false);
  }
}

export const stopInstall = () => void api.cancelGalleryInstall();

/** The plain words for a failed list, or with `install` a failed install. */
export function galleryError(text: string, install = false): { title: string; detail: string; retry: boolean } {
  const code = text.match(/^[A-Z][A-Z_]+(?=:)/)?.[0] ?? "";
  const detail = code ? text.slice(code.length + 1).trim() : text;
  if (code === "OFFLINE") return { title: "Can’t reach the gallery", detail: "Check your internet connection.", retry: true };
  if (code === "GALLERY_BUSY") return { title: "GitHub is busy", detail: "Try again in a few minutes.", retry: true };
  if (code === "GALLERY_FAILED") return { title: "The gallery isn’t answering", detail: "Try again later.", retry: true };
  if (code === "GALLERY_NEWER") return { title: "This gallery needs a newer Nuzky", detail: "", retry: false };
  if (code === "PLUGIN_API_NEWER") return { title: "This plugin needs a newer Nuzky", detail: "", retry: false };
  if (code === "GALLERY_GONE") return { title: "No longer in the gallery", detail: "", retry: false };
  if (code === "GALLERY_STALE") return { title: "The gallery changed", detail: "Check what this plugin does now, then try again.", retry: false };
  if (code === "GALLERY_CHANGED") return { title: "Your copy changed", detail: "Check what Replace will do now, then try again.", retry: false };
  if (code === "GALLERY_INSTALLING") return { title: "Another plugin is installing", detail: "Wait for it to finish.", retry: true };
  if (code === "GALLERY_REFUSED") return { title: "Nothing was installed", detail: REFUSED, retry: false };
  if (code === "GALLERY_BROKEN") return { title: "Nothing was installed", detail, retry: false };
  if (code === "GALLERY_CORRUPT")
    return install ? { title: "Nothing was installed", detail: "A file didn’t download correctly. Try again.", retry: true } : { title: "The gallery didn’t load", detail: "Try again later.", retry: true };
  if (code.startsWith("STORE_")) return { title: "Couldn’t save the plugin", detail, retry: true };
  return { title: "Something went wrong", detail, retry: true };
}

export function size(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1_048_576) return `${Math.round(bytes / 1024)} KB`;
  return `${(bytes / 1_048_576).toFixed(1)} MB`;
}

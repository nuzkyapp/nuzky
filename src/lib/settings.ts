import { create } from "zustand";

export const PANES = ["general", "ai", "models", "sounds", "plugins", "keyboard", "storage", "privacy"] as const;
export type Pane = (typeof PANES)[number];

export const PANE_TITLES: Record<Pane, string> = {
  general: "General",
  ai: "AI",
  models: "Models",
  sounds: "Sounds & YouTube",
  plugins: "Plugins",
  keyboard: "Keyboard",
  storage: "Storage",
  privacy: "Privacy",
};

const PANE_KEY = "nuzky.settingsPane";

function savedPane(): Pane {
  const v = localStorage.getItem(PANE_KEY);
  return (PANES as readonly string[]).includes(v ?? "") ? (v as Pane) : "general";
}

type Settings = {
  open: boolean;
  pane: Pane;
  /** The `data-setting` of the row to scroll to and light up once the pane shows. */
  target: string | null;
};

export const useSettings = create<Settings>(() => ({ open: false, pane: savedPane(), target: null }));

useSettings.subscribe((s, prev) => {
  if (s.pane !== prev.pane) localStorage.setItem(PANE_KEY, s.pane);
});

/** Opens Settings on `pane`, or on the pane shown last, as Apple's settings windows do. */
export function openSettings(pane?: Pane, target: string | null = null) {
  useSettings.setState((s) => ({ open: true, pane: pane ?? s.pane, target }));
}

export const closeSettings = () => useSettings.setState({ open: false, target: null });

/** Whether `pane` is on screen now, so what happens there needs no toast. */
export const paneShown = (pane: Pane) => {
  const s = useSettings.getState();
  return s.open && s.pane === pane;
};

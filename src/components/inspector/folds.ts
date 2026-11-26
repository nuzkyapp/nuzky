import { create } from "zustand";
import type { Fold } from "../ui";

export type FoldId = "transform" | "shape" | "mask" | "background" | "motion" | "finish";

/** Which sections of a clip's inspector are open, kept while another clip is selected. Transform starts open. */
const useFolds = create<Record<FoldId, boolean>>(() => ({ transform: true, shape: false, mask: false, background: false, motion: false, finish: false }));

export function useFold(id: FoldId, summary: string, set: boolean): Fold {
  const open = useFolds((s) => s[id]);
  return { open, summary, set, onToggle: () => useFolds.setState({ [id]: !open }) };
}

/** "Rounded, cropped" from ["rounded", "cropped"]; "None" when nothing is set. */
export const summaryOf = (parts: string[]) => (parts.length === 0 ? "None" : parts.join(", ").replace(/^./, (c) => c.toUpperCase()));

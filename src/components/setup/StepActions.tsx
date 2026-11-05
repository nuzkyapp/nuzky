import { createContext, useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const StepActionsSlot = createContext<HTMLElement | null>(null);

/** A step shown as a part of Settings rather than in Get set up: its text is quieter and its download is no prominent action. */
export const InSettings = createContext(false);
export const useInSettings = () => useContext(InSettings);

/** A step's own buttons, in the dialog's footer before Skip, Next or Done, so they stay in view while the step scrolls. */
export function StepActions({ children }: { children: ReactNode }) {
  const slot = useContext(StepActionsSlot);
  return slot && createPortal(children, slot);
}

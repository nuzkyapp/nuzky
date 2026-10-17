import { createContext, useContext, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const StepActionsSlot = createContext<HTMLElement | null>(null);

/** A step's own buttons, in the dialog's footer before Skip, Next or Done, so they stay in view while the step scrolls. */
export function StepActions({ children }: { children: ReactNode }) {
  const slot = useContext(StepActionsSlot);
  return slot && createPortal(children, slot);
}

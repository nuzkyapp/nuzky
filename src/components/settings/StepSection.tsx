import { useState, type ReactNode } from "react";
import { InSettings, StepActionsSlot } from "../setup/StepActions";

/** A step of Get set up as a part of a Settings pane, with its own buttons under it instead of in the setup's footer. */
export function StepSection({ title, id, children }: { title: string; id: string; children: ReactNode }) {
  const [actions, setActions] = useState<HTMLElement | null>(null);
  return (
    <section data-setting={id} aria-label={title} className="flex flex-col rounded-[14px]">
      <h3 className="text-[13px] font-semibold text-fg">{title}</h3>
      <InSettings.Provider value={true}>
        <StepActionsSlot.Provider value={actions}>{children}</StepActionsSlot.Provider>
      </InSettings.Provider>
      <div ref={setActions} className="mt-3 flex justify-end gap-2 empty:hidden" />
    </section>
  );
}

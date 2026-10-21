import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { Check, X } from "lucide-react";
import { SETUP_STEPS, useSetup } from "../../lib/setup";
import { trySample, useTour } from "../../lib/tour";
import { Button, IconButton, trapTab } from "../ui";
import { StepActionsSlot } from "./StepActions";
import { STEPS } from "./steps";

const LAST = SETUP_STEPS.length - 1;

/** Get set up: the setup steps beside each other with whether each is done or skipped; it continues where it was left. */
export function SetupDialog() {
  const { open, states, current, closeSetup, showStep, skipStep } = useSetup();
  const rail = useRef<HTMLDivElement>(null);
  const [actions, setActions] = useState<HTMLElement | null>(null);
  // Opened by itself on the first launch, its last step goes on to the sample instead of closing.
  const firstSetup = useTour((s) => s.firstSetup);
  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    // Never Skip: a stray Enter or Space skips nothing.
    rail.current?.querySelector<HTMLElement>("[aria-selected=true]")?.focus();
    return () => opener?.focus();
  }, [open]);

  if (!open) return null;

  const index = SETUP_STEPS.indexOf(current);
  const done = SETUP_STEPS.filter((step) => states[step] === "done").length;
  const { title, Body } = STEPS[current];
  const toSample = firstSetup && index === LAST && !!states[current];

  const railKeys = (e: KeyboardEvent<HTMLDivElement>) => {
    const to = { ArrowUp: index - 1, ArrowDown: index + 1, Home: 0, End: LAST }[e.key];
    if (to === undefined) return;
    e.preventDefault();
    const step = SETUP_STEPS[Math.max(0, Math.min(LAST, to))];
    showStep(step);
    e.currentTarget.querySelector<HTMLElement>(`#setup-tab-${step}`)?.focus();
  };

  return (
    <div
      data-scrim
      className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/45"
      onPointerDown={(e) => {
        if (e.target !== e.currentTarget) return;
        // Otherwise the press would take the focus that closing gives back to the opener.
        e.preventDefault();
        closeSetup();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="setup-title"
        aria-describedby="setup-count"
        // A click on its text keeps the focus inside, so the keys below still see it.
        tabIndex={-1}
        onKeyDown={(e) => {
          // The editor's and the home screen's keys (Space plays, typing searches) wait while it is open; F8 still reaches the toasts.
          if (e.key !== "F8") e.stopPropagation();
          if (e.key === "Escape") closeSetup();
          else trapTab(e);
        }}
        className="dialog-in overlay flex h-[560px] max-h-[calc(100vh-48px)] w-[640px] flex-col rounded-[20px] p-6 text-fg outline-none"
      >
        <div className="flex items-start justify-between gap-3">
          <div>
            <h2 id="setup-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
              Get set up
            </h2>
            <p id="setup-count" className="tabular mt-0.5 text-[13px] text-muted">
              {done} of {SETUP_STEPS.length} done
            </p>
          </div>
          <IconButton label="Close" round className="-mr-1.5 -mt-0.5 h-7 w-7" onClick={closeSetup}>
            <X size={16} />
          </IconButton>
        </div>
        <div className="mt-5 flex min-h-0 flex-1">
          <div ref={rail} role="tablist" aria-orientation="vertical" aria-label="Setup steps" onKeyDown={railKeys} className="-m-1 flex w-[204px] shrink-0 flex-col gap-0.5 overflow-y-auto p-1">
            {SETUP_STEPS.map((step, i) => {
              const state = states[step];
              const on = step === current;
              return (
                <button
                  key={step}
                  type="button"
                  role="tab"
                  id={`setup-tab-${step}`}
                  aria-selected={on}
                  aria-controls="setup-panel"
                  tabIndex={on ? 0 : -1}
                  onClick={() => showStep(step)}
                  className={`flex shrink-0 items-start gap-2.5 rounded-lg px-2.5 py-1.5 text-left transition-colors duration-[120ms] ease-out ${on ? "bg-white/[.08]" : "hover:bg-white/[.05]"}`}
                >
                  <span
                    className={`tabular flex size-5 shrink-0 items-center justify-center rounded-full text-[11px] font-semibold ${
                      state === "done" ? "bg-accent-strong text-white" : state === "skipped" ? "text-muted shadow-[inset_0_0_0_1.5px_rgb(255_255_255/.25)]" : "bg-white/[.09] text-fg"
                    }`}
                  >
                    {state === "done" ? <Check size={12} strokeWidth={3} /> : i + 1}
                  </span>
                  <span className="min-w-0 pt-px">
                    <span className={`block text-[13px] text-fg ${on ? "font-medium" : ""}`}>{STEPS[step].title}</span>
                    {state && <span className="block text-[12px] text-muted">{state === "done" ? "Done" : "Skipped"}</span>}
                  </span>
                </button>
              );
            })}
          </div>
          <div className="mx-4 w-px shrink-0 bg-white/[.07]" />
          <div role="tabpanel" id="setup-panel" aria-labelledby={`setup-tab-${current}`} className="flex min-w-0 flex-1 flex-col">
            <h3 className="text-[15px] font-semibold">{title}</h3>
            {/* 4 px each side, so focus rings at the edges are not cut off. */}
            <div className="-mx-1 flex min-h-0 flex-1 flex-col overflow-y-auto px-1">
              <StepActionsSlot.Provider value={actions}>
                <Body />
              </StepActionsSlot.Provider>
            </div>
            <footer className="mt-4 flex justify-end gap-2">
              {/* Left of the step's own buttons, which the steps' focus finds by `footer button:not([data-sample])`. */}
              {!toSample && (
                <Button variant="ghost" data-sample className="mr-auto -ml-3 text-accent! hover:text-accent!" onClick={trySample}>
                  Try the sample video
                </Button>
              )}
              <span ref={setActions} className="contents" />
              {!states[current] ? (
                <Button pill onClick={() => skipStep(current)}>
                  Skip
                </Button>
              ) : index < LAST ? (
                <Button pill variant="primary" onClick={() => showStep(SETUP_STEPS[index + 1])}>
                  Next
                </Button>
              ) : toSample ? (
                <Button pill variant="primary" onClick={trySample}>
                  Try the sample video
                </Button>
              ) : (
                // A skipped step's own way on, such as Open a video…, is the prominent one.
                <Button pill variant={states[current] === "skipped" ? "secondary" : "primary"} onClick={closeSetup}>
                  Done
                </Button>
              )}
            </footer>
          </div>
        </div>
      </div>
    </div>
  );
}

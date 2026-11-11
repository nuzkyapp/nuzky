import { useEffect, useRef, type KeyboardEvent } from "react";
import { Check } from "lucide-react";
import { PRESETS, useKeys, type PresetId } from "../lib/keys";
import { answerKeysQuestion, useKeysQuestion } from "../lib/keysQuestion";
import { Keycap } from "./settings/KeysPane";
import { Button, trapTab } from "./ui";

/** The editors in the order people asked for them; Nuzky's own keys first. */
const CHOICES: PresetId[] = ["nuzky", "capcut", "finalCut", "resolve", "premiere"];

/** Which editor's keys to use: one click puts a set in place, Not now keeps the keys as they are. */
export function KeysQuestionDialog() {
  const open = useKeysQuestion((s) => s.open);
  const { preset, changed } = useKeys();
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (!open) return void (d.open && d.close());
    if (d.open) return;
    d.showModal();
    // It opens unasked, so the dialog takes focus rather than a choice: a key typed meanwhile chooses nothing.
    d.focus();
  }, [open]);

  const arrows = (e: KeyboardEvent<HTMLDialogElement>) => {
    const rows = [...e.currentTarget.querySelectorAll<HTMLButtonElement>("[data-keys-choice]")];
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    const to = { ArrowDown: at < 0 ? rows.findIndex((r) => r.dataset.keysChoice === preset) : at + 1, ArrowUp: at < 0 ? rows.length - 1 : at - 1, Home: 0, End: rows.length - 1 }[e.key];
    if (to === undefined) return false;
    e.preventDefault();
    rows[Math.max(0, Math.min(rows.length - 1, to))]?.focus();
    return true;
  };
  const dropped = Object.keys(changed).length;

  return (
    <dialog
      data-exit
      data-keys-question
      ref={dialog}
      tabIndex={-1}
      aria-labelledby="keys-question-title"
      aria-describedby="keys-question-text"
      onCancel={(e) => {
        e.preventDefault();
        answerKeysQuestion(null);
      }}
      onPointerDown={(e) => {
        // A press on the backdrop lands on the dialog itself, outside its box.
        const b = e.currentTarget.getBoundingClientRect();
        if (e.target === e.currentTarget && (e.clientX < b.left || e.clientX > b.right || e.clientY < b.top || e.clientY > b.bottom)) answerKeysQuestion(null);
      }}
      onKeyDown={(e) => {
        // The editor's keys wait until it is answered; F8 still reaches the toasts.
        if (e.key !== "F8") e.stopPropagation();
        if (!arrows(e)) trapTab(e);
      }}
      className="dialog-in overlay overlay-solid fixed inset-0 m-auto w-[440px] rounded-[20px] p-6 text-fg outline-none backdrop:bg-black/45"
    >
      {open && (
        <>
          <h2 id="keys-question-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Which editor's keys do you know?
          </h2>
          <p id="keys-question-text" className="mt-2 text-[13px] text-fg/85">
            Nuzky can use the same keys. Settings › Keyboard changes them any time.
          </p>
          <div role="group" aria-label="Keys like" className="mt-4 rounded-xl bg-white/[.055] px-1 py-1 shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
            {CHOICES.map((id) => {
              const split = PRESETS[id].keys.split[0];
              return (
                <button
                  key={id}
                  type="button"
                  data-keys-choice={id}
                  aria-current={id === preset || undefined}
                  onClick={() => answerKeysQuestion(id)}
                  className="flex h-10 w-full items-center gap-2 rounded-lg px-2.5 text-left transition-colors duration-[120ms] ease-out hover:bg-white/[.06] focus-visible:bg-white/[.08]"
                >
                  <span className="flex w-4 shrink-0 justify-center">{id === preset && <Check size={14} strokeWidth={2.5} className="text-accent" />}</span>
                  <span className="min-w-0 flex-1 truncate text-[13px] font-medium">{PRESETS[id].name}</span>
                  {split && (
                    <span className="flex shrink-0 items-center gap-1 text-[12px] text-muted">
                      <Keycap chord={split} changed={false} or={false} />
                      <span className="pl-0.5">splits</span>
                    </span>
                  )}
                </button>
              );
            })}
          </div>
          {dropped > 0 && (
            <p className="mt-3 text-[12px] text-warn">
              Choosing a set replaces {dropped === 1 ? "your 1 changed key" : `your ${dropped} changed keys`}.
            </p>
          )}
          <div className="mt-6 flex justify-end">
            <Button pill onClick={() => answerKeysQuestion(null)}>
              Not now
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}

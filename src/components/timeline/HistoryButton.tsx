import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Redo2, Sparkles, Undo2 } from "lucide-react";
import { api } from "../../lib/api";
import { currentEpoch, useEditor } from "../../lib/store";
import type { HistoryPeek } from "../../lib/types";
import { IconButton } from "../ui";

export type Peek = HistoryPeek & { redo: boolean };

/** How long the pointer rests on the button before the timeline shows the step, so passing over it never flickers. */
const PEEK_DELAY_MS = 300;

/**
 * Undo or Redo. Resting the pointer on it, or reaching it with the keyboard, reads the project the step would
 * restore and hands it to the timeline (`onPeek`), which draws it; `lines` say in words what changes. Nothing
 * changes until the click. After a click the timeline shows the result; the next step shows once the pointer
 * or focus comes back.
 */
export function HistoryButton({ redo, locked, lines, onPeek }: { redo: boolean; locked: boolean; lines: string[] | null; onPeek: (peek: Peek | null) => void }) {
  const can = useEditor((s) => !!(redo ? s.snap?.canRedo : s.snap?.canUndo));
  const revision = useEditor((s) => s.snap?.revision);
  const [pointer, setPointer] = useState(false);
  const [keyboard, setKeyboard] = useState(false);
  // A click or Esc hides the preview until the pointer or focus leaves.
  const [dismissed, setDismissed] = useState(false);
  const [shown, setShown] = useState<Peek | null>(null);
  const ref = useRef<HTMLButtonElement>(null);
  const tipId = useId();
  const live = can && !locked && !dismissed && (pointer || keyboard);

  useEffect(() => {
    if (!live) return;
    let current = true;
    const timer = setTimeout(
      async () => {
        const found = await api.peekHistory(currentEpoch(), redo).catch((error) => {
          console.warn("peek_history", error);
          return null;
        });
        if (!current || !found || found.revision !== useEditor.getState().snap?.revision) return;
        const peek = { ...found, redo };
        setShown(peek);
        onPeek(peek);
      },
      keyboard ? 0 : PEEK_DELAY_MS,
    );
    return () => {
      current = false;
      clearTimeout(timer);
      setShown(null);
      onPeek(null);
    };
  }, [live, revision, redo, keyboard, onPeek]);

  // Esc hides the preview wherever focus is, before it would clear the selection; an open menu or dialog keeps it.
  useEffect(() => {
    if (!shown) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape" || document.querySelector("[role=menu], [role=dialog]")) return;
      e.stopPropagation();
      setDismissed(true);
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [shown]);

  const name = redo ? "Redo" : "Undo";
  const at = shown && lines && ref.current?.getBoundingClientRect();
  return (
    <>
      <IconButton
        ref={ref}
        round
        label={locked ? `${name} is available when the AI is done` : !can ? `Nothing to ${name.toLowerCase()}` : redo ? "Redo (Ctrl+Shift+Z)" : "Undo (Ctrl+Z)"}
        disabled={locked || !can}
        onClick={() => {
          setDismissed(true);
          void useEditor.getState()[redo ? "redo" : "undo"]();
        }}
        // The tip below says more than the native tooltip would.
        {...(at ? { title: "" } : {})}
        aria-describedby={at ? tipId : undefined}
        data-peek-button={redo ? "redo" : "undo"}
        onPointerEnter={() => setPointer(true)}
        onPointerLeave={() => {
          setPointer(false);
          setDismissed(false);
        }}
        onFocus={(e) => setKeyboard(e.currentTarget.matches(":focus-visible"))}
        onBlur={() => {
          setKeyboard(false);
          setDismissed(false);
        }}
      >
        {redo ? <Redo2 size={16} /> : <Undo2 size={16} />}
      </IconButton>
      {at &&
        lines &&
        createPortal(
          <div
            id={tipId}
            role="tooltip"
            data-testid="history-peek"
            // Solid, not glass: it opens over the toasts and the preview, whose text would show through.
            className="pointer-events-none fixed z-[130] w-max max-w-[320px] -translate-y-full rounded-lg bg-raised px-3 py-2 text-[12px] text-fg shadow-[0_8px_24px_rgb(0_0_0/.5)] inset-ring inset-ring-white/10"
            style={{ left: at.left, top: at.top - 8 }}
          >
            <div className="flex items-center gap-1.5 font-semibold">
              {shown.ai && <Sparkles size={12} className="shrink-0 text-accent" />}
              <span className="truncate">
                {shown.ai ? `${name} AI run` : name}
                {shown.label && ` · ${shown.label}`}
              </span>
              <span className="ml-auto shrink-0 pl-3 font-normal text-muted">{redo ? "Ctrl+Shift+Z" : "Ctrl+Z"}</span>
            </div>
            {lines.map((line, i) => (
              <div key={i} className="mt-0.5 text-fg/85">
                {line}
              </div>
            ))}
          </div>,
          document.body,
        )}
    </>
  );
}

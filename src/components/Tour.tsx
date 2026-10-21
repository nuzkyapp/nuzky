import { useEffect, useLayoutEffect, useRef } from "react";
import { agentName, useAgent, type AgentId } from "../lib/agent";
import { planReady, useChatGpt } from "../lib/chatgpt";
import { usePreview } from "../lib/preview";
import { useSetup } from "../lib/setup";
import { useEditor } from "../lib/store";
import { STEP, TOUR_STEPS, changed, endTour, nextStep, openOwnVideo, useTour, type TourStep } from "../lib/tour";
import { Button } from "./ui";

/** Space between the target and the hole's edge, the hole and the card, and the card and the window's edges. */
const PAD = 4;
const GAP = 12;
const MARGIN = 12;

type Hole = { x: number; y: number; w: number; h: number; r: number };

const radii = new WeakMap<Element, number>();

/** The hole around an element: a little larger, with the corners of its shape; null when it is not on screen. */
function holeOf(el: Element | null): Hole | null {
  if (!el) return null;
  const b = el.getBoundingClientRect();
  if (b.width === 0 || b.height === 0) return null;
  if (!radii.has(el)) radii.set(el, parseFloat(getComputedStyle(el).borderTopLeftRadius) || 0);
  // Kept 2 px inside the window, so the ring around it shows whole.
  const x = Math.max(2, b.left - PAD);
  const y = Math.max(2, b.top - PAD);
  const w = Math.min(window.innerWidth - 2, b.right + PAD) - x;
  const h = Math.min(window.innerHeight - 2, b.bottom + PAD) - y;
  return { x, y, w, h, r: Math.min(radii.get(el)! + PAD, w / 2, h / 2) };
}

const rounded = ({ x, y, w, h, r }: Hole) =>
  `M${x + r} ${y}H${x + w - r}A${r} ${r} 0 0 1 ${x + w} ${y + r}V${y + h - r}A${r} ${r} 0 0 1 ${x + w - r} ${y + h}H${x + r}A${r} ${r} 0 0 1 ${x} ${y + h - r}V${y + r}A${r} ${r} 0 0 1 ${x + r} ${y}Z`;

const last = (selector: string | undefined) => (selector ? ([...document.querySelectorAll(selector)].pop() ?? null) : null);

/**
 * Where the card goes: on the step's side of the hole when it fits there, else on another side, else in the middle;
 * always inside the window, and off the video where a side lets it, so a playing video is never covered.
 */
function place(hole: Hole | null, side: (typeof STEP)[TourStep]["side"], w: number, h: number, video: DOMRect | null) {
  const W = window.innerWidth;
  const H = window.innerHeight;
  const clamp = (v: number, size: number, room: number) => Math.max(MARGIN, Math.min(room - MARGIN - size, v));
  if (!hole) return { x: clamp((W - w) / 2, w, W), y: clamp((H - h) / 2, h, H) };
  const across = clamp(hole.x + hole.w / 2 - w / 2, w, W);
  const down = clamp(hole.y + hole.h / 2 - h / 2, h, H);
  const spots = {
    left: hole.x - GAP - w >= MARGIN && { x: hole.x - GAP - w, y: down },
    right: hole.x + hole.w + GAP + w <= W - MARGIN && { x: hole.x + hole.w + GAP, y: down },
    above: hole.y - GAP - h >= MARGIN && { x: across, y: hole.y - GAP - h },
    below: hole.y + hole.h + GAP + h <= H - MARGIN && { x: across, y: hole.y + hole.h + GAP },
  };
  const fitting = [spots[side], spots.right, spots.left, spots.below, spots.above].filter((spot) => spot !== false);
  const clear = ({ x, y }: { x: number; y: number }) => !video || x >= video.right || x + w <= video.left || y >= video.bottom || y + h <= video.top;
  return fitting.find(clear) ?? fitting[0] ?? { x: clamp((W - w) / 2, w, W), y: clamp((H - h) / 2, h, H) };
}

const FOCUSABLE = "button, input, select, textarea, a[href], [tabindex]:not([tabindex='-1'])";
const focusables = (root: Element | null) =>
  root ? [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter((el) => !el.matches(":disabled") && el.getClientRects().length > 0) : [];

/**
 * Something the tour does not own is over the editor and owns Esc and Tab now: a dialog such as the launcher, a menu, or
 * the full screen preview, which covers every part but the preview. The tour steps aside meanwhile and comes back after.
 */
const somethingOver = (step: TourStep) => !!document.querySelector("dialog[open], [aria-modal='true'], [role=menu]") || (usePreview.getState().full && step !== "play");

/**
 * The tour over the editor: everything but the step's target lies under a dim that takes the clicks, and a card next to
 * the target says what to do. The hole follows its target as the layout changes.
 */
export function Tour() {
  const step = useTour((s) => s.step);
  return step ? <Spotlight step={step} /> : null;
}

function Spotlight({ step }: { step: TourStep }) {
  const def = STEP[step];
  const stopped = useTour((s) => s.stopped);
  const undone = useTour((s) => s.undone);
  const word = useTour((s) => s.word);
  const ai = useRealAi();
  const cut = useAgent((s) => changed(s.items));
  const layer = useRef<SVGSVGElement>(null);
  const dim = useRef<SVGPathElement>(null);
  const rings = useRef<SVGRectElement[]>([]);
  const card = useRef<HTMLDivElement>(null);

  // Read from the layout every frame, written only when something moved, so the hole follows resizing, scrolling and
  // panels moving without a render.
  useLayoutEffect(() => {
    let frame = 0;
    let drawn = "";
    let focus = def.focus ?? null;
    let shown = false;
    const tick = () => {
      frame = requestAnimationFrame(tick);
      const visibility = somethingOver(step) ? "hidden" : "";
      for (const el of [layer.current, card.current]) if (el && el.style.visibility !== visibility) el.style.visibility = visibility;
      const target = last(def.target);
      if (target && !shown) {
        shown = true;
        target.scrollIntoView({ block: "nearest" });
      }
      const wanted = focus && last(focus);
      if (wanted instanceof HTMLElement) {
        focus = null;
        wanted.focus();
      }
      const holes = [holeOf(target), holeOf(last(def.extra))];
      const box = card.current;
      if (!box || !dim.current) return;
      const video = document.querySelector("section[aria-label=Preview] canvas")?.getBoundingClientRect() ?? null;
      const key = `${JSON.stringify([holes, video])} ${window.innerWidth} ${window.innerHeight} ${box.offsetWidth} ${box.offsetHeight}`;
      if (key === drawn) return;
      drawn = key;
      dim.current.setAttribute("d", `M0 0H${window.innerWidth}V${window.innerHeight}H0Z${holes.map((h) => (h ? rounded(h) : "")).join("")}`);
      holes.forEach((h, i) => {
        const ring = rings.current[i];
        if (!ring) return;
        ring.style.display = h ? "" : "none";
        if (!h) return;
        // A hairline just outside the hole, centred on its edge half a pixel out.
        ring.setAttribute("x", String(h.x - 0.5));
        ring.setAttribute("y", String(h.y - 0.5));
        ring.setAttribute("width", String(h.w + 1));
        ring.setAttribute("height", String(h.h + 1));
        ring.setAttribute("rx", String(h.r + 0.5));
      });
      const at = place(holes[0], def.side, box.offsetWidth, box.offsetHeight, video);
      box.style.left = `${Math.round(at.x)}px`;
      box.style.top = `${Math.round(at.y)}px`;
    };
    tick();
    return () => cancelAnimationFrame(frame);
  }, [def, step]);

  // The card takes the keyboard unless the step's action is on the target; Space then plays, as everywhere in the editor.
  useEffect(() => {
    if (def.focus) return;
    // After the frame, so a panel the step opens again, whose field takes the focus as it appears, does not keep it.
    const frame = requestAnimationFrame(() => {
      const box = card.current;
      (box?.querySelector<HTMLElement>("[data-primary]") ?? box)?.focus();
    });
    return () => cancelAnimationFrame(frame);
  }, [def, stopped, undone]);

  // Esc ends the tour; Tab goes round the card and the target, as clicks reach only them.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (somethingOver(step)) return;
      if (e.key === "Escape") {
        // The full screen preview takes Esc first.
        if (usePreview.getState().full) return;
        e.preventDefault();
        e.stopPropagation();
        endTour();
      } else if (e.key === "Tab") {
        const items = [...focusables(card.current), ...focusables(last(def.target)), ...focusables(last(def.extra))];
        if (!items.length) return;
        e.preventDefault();
        const i = items.indexOf(document.activeElement as HTMLElement);
        items[i < 0 ? (e.shiftKey ? items.length - 1 : 0) : (i + (e.shiftKey ? items.length - 1 : 1)) % items.length].focus();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [def, step]);

  const index = TOUR_STEPS.indexOf(step);
  const text =
    step === "watch" && stopped
      ? cut
        ? "You stopped it. What it changed so far stays, and Undo on its card takes it back."
        : "You stopped it before it changed anything."
      : step === "undo" && undone
        ? "Undone. The video is back as it was. Redo, or Ctrl+Shift+Z, brings the edit back."
        : step === "transcript" && word
          ? `The video starts on “${word}”. Press Delete to cut it.`
          : def.text;
  const next = def.next || (step === "watch" && stopped);
  return (
    <>
      <svg ref={layer} aria-hidden className="scrim-in pointer-events-none fixed inset-0 z-[85] h-full w-full">
        <path ref={dim} fillRule="evenodd" fill="rgb(0 0 0 / 0.62)" style={{ pointerEvents: "auto" }} />
        {[0, 1].map((i) => (
          <rect key={i} ref={(el) => void (el && (rings.current[i] = el))} fill="none" strokeWidth={1} stroke="rgb(255 255 255 / 0.24)" style={{ display: "none" }} />
        ))}
      </svg>
      <div
        key={step}
        ref={card}
        role="dialog"
        aria-labelledby="tour-title"
        aria-describedby="tour-text"
        tabIndex={-1}
        className="dialog-in overlay fixed z-[95] w-[300px] rounded-[14px] p-4 text-fg outline-none"
      >
        <p className="tabular text-[12px] leading-4 text-muted">
          {index + 1} of {TOUR_STEPS.length}
        </p>
        <h2 id="tour-title" className="mt-1 text-[15px] font-semibold leading-5">
          {def.title}
        </h2>
        <div id="tour-text" aria-live="polite" className="mt-1 flex flex-col gap-2 text-[13px] leading-[18px] text-fg/85">
          <p>{text}</p>
          {step === "export" && <DemoNote ai={ai} />}
        </div>
        <div className="mt-3 flex items-center gap-2">
          {step !== "export" && (
            <Button variant="ghost" className="-ml-2 h-7 px-2 text-[12px]" onClick={endTour}>
              Skip tour
            </Button>
          )}
          <span className="flex-1" />
          {step === "undo" && undone && (
            <Button pill className="h-7 px-3 text-[12px]" onClick={() => void useEditor.getState().redo()}>
              Redo
            </Button>
          )}
          {next && (
            <Button variant="primary" pill data-primary className="h-7 px-3 text-[12px]" onClick={nextStep}>
              Next
            </Button>
          )}
          {step === "export" && (
            <>
              {ai === null && !WINDOWS ? (
                <Button pill className="h-7 px-3 text-[12px]" onClick={connectAi}>
                  Connect your AI
                </Button>
              ) : (
                <Button pill className="h-7 px-3 text-[12px]" onClick={endTour}>
                  Done
                </Button>
              )}
              <Button variant="primary" pill data-primary className="h-7 px-3 text-[12px]" onClick={openOwnVideo}>
                Open my own video…
              </Button>
            </>
          )}
        </div>
      </div>
    </>
  );
}

/** The AI that edits for real after the tour: the panel's own when it can answer, else one that can; null without one,
 * undefined while Nuzky is still looking. */
function useRealAi() {
  const agents = useAgent((s) => s.agents);
  const agent = useAgent((s) => s.agent);
  const chatgpt = useChatGpt((s) => planReady(s.status));
  if (agents === null) return undefined;
  const can = (id: AgentId) => (id === "chatgpt" ? chatgpt : !!agents.find((a) => a.id === id)?.path);
  const id = can(agent) ? agent : (agents.find((a) => a.path)?.id ?? (chatgpt ? "chatgpt" : null));
  return id && agentName(id, agents);
}

const WINDOWS = navigator.userAgent.includes("Windows");

/** The last step always says the tour's AI was a demo, and what a real one does here. */
function DemoNote({ ai }: { ai: string | null | undefined }) {
  return (
    <p className="text-[12px] leading-4 text-muted">
      The AI in this tour was a demo.
      {WINDOWS ? " AI editing isn't available on Windows yet." : ai ? ` ${ai} does it for real on your videos. Next time, type what you want in the AI panel.` : ""}
    </p>
  );
}

/** Without an AI, Get set up's AI step, where one is connected. */
function connectAi() {
  endTour();
  useSetup.getState().showStep("ai");
  useSetup.getState().openSetup();
}

import { Fragment, useEffect, useRef, useState, type KeyboardEvent, type ReactNode } from "react";
import { Check, ChevronDown, RectangleHorizontal } from "lucide-react";
import { chose } from "../../lib/exit";
import { FORMATS, canvasSize, formatLabel, sameRatio } from "../../lib/presets";
import { useFollow } from "../../lib/follow";
import { layOut, picturedClips, reframe, useReframe } from "../../lib/reframe";
import { AI_EDITING, useAiLocked, useEditor } from "../../lib/store";
import { Button, PresetTile } from "../ui";

type Style = "fill" | "fade" | "stack";

/** A vertical frame drawn as each layout fills it. */
const STYLES: { style: Style; label: string; title: (which: string) => string; frame: ReactNode }[] = [
  {
    style: "fill",
    label: "Fill",
    title: (which) => `Fill the frame and keep the speaker's face in ${which}`,
    frame: <span className="absolute inset-0 bg-fg/70" />,
  },
  {
    style: "fade",
    label: "Fade",
    title: (which) => `Show ${which} whole across the middle, a blurred copy behind`,
    frame: (
      <>
        <span className="absolute inset-0 bg-fg/25" />
        <span className="absolute inset-x-0 top-1/2 h-[7px] -translate-y-1/2 bg-fg/70" />
      </>
    ),
  },
  {
    style: "stack",
    label: "Stack",
    title: (which) => `Show ${which} twice, one above the other`,
    frame: (
      <>
        <span className="absolute inset-x-0 top-0 h-[calc(50%-0.5px)] bg-fg/70" />
        <span className="absolute inset-x-0 bottom-0 h-[calc(50%-0.5px)] bg-fg/70" />
      </>
    ),
  },
];

/**
 * CapCut's "Ratio" control next to the player: the one place to change the canvas format, and beside each format
 * Reframe, which also places the clips to follow the face, or on a vertical format Layout, whose Fill is Reframe and
 * whose Fade and Stack show a wide picture whole or twice. A format keeps the resolution, which the quality chip in
 * the top bar sets. Open state lives in the store so the Format row in the Project inspector can open it too.
 */
export function RatioMenu() {
  const canvas = useEditor((s) => s.snap!.project.canvas);
  const pictures = useEditor((s) => s.snap!.project.tracks.some((t) => t.kind === "video" && t.clips.some((c) => c.content.type === "media")));
  const selected = useEditor((s) => picturedClips(s.snap!.project, s.selection).length);
  const reframing = useReframe((s) => !!s.job);
  const following = useFollow((s) => !!s.job);
  const edit = useEditor((s) => s.edit);
  const open = useEditor((s) => s.ratioOpen);
  const locked = useAiLocked();
  const short = Math.min(canvas.width, canvas.height);
  const setOpen = (ratioOpen: boolean) => useEditor.setState({ ratioOpen });
  const [layouts, setLayouts] = useState<string | null>(null);
  const ref = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const closeToTrigger = () => {
    setOpen(false);
    trigger.current?.focus();
  };

  useEffect(() => {
    if (layouts) ref.current?.querySelector<HTMLElement>("[data-layouts] [role=menuitem]")?.focus();
  }, [layouts]);

  useEffect(() => {
    if (!open) return setLayouts(null);
    ref.current?.querySelector<HTMLElement>("[aria-checked=true]")?.focus();
    const close = (e: PointerEvent) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const key = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeToTrigger();
      }
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", key, true);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", key, true);
    };
  }, [open]);

  // Like the clip menu: ↑/↓ move through each format, its Reframe or Layout and Layout's styles, and no key reaches the
  // editor's shortcuts.
  const onMenuKey = (e: KeyboardEvent<HTMLDivElement>) => {
    e.stopPropagation();
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const items = [...e.currentTarget.querySelectorAll<HTMLElement>("[role=menuitemradio], [role=menuitem]")];
    const i = items.indexOf(document.activeElement as HTMLElement);
    items[(i + (e.key === "ArrowDown" ? 1 : items.length - 1)) % items.length]?.focus();
  };

  return (
    <div className="relative" ref={ref}>
      <button
        ref={trigger}
        type="button"
        data-ratio
        aria-haspopup="menu"
        aria-expanded={open}
        title={locked ? AI_EDITING : "Canvas ratio"}
        disabled={locked}
        onClick={() => setOpen(!open)}
        className={`flex h-8 items-center gap-1.5 rounded-full px-3 text-[12px] font-medium transition-colors duration-[120ms] ease-out disabled:cursor-not-allowed disabled:opacity-40 ${
          open ? "bg-white/[.14] text-fg" : "text-fg enabled:hover:bg-white/[.08]"
        }`}
      >
        <RectangleHorizontal size={14} className="text-muted @max-[560px]:hidden" />
        <span className="@max-[560px]:hidden">Ratio</span>
        <span className="tabular font-normal whitespace-nowrap text-muted">{formatLabel(canvas.width, canvas.height)}</span>
        <ChevronDown size={13} className="text-muted" />
      </button>
      {open && (
        <div data-exit role="menu" aria-label="Canvas ratio" onKeyDown={onMenuKey} className="overlay menu-in absolute bottom-11 right-0 z-50 w-[19rem] origin-bottom-right rounded-xl p-1">
          {FORMATS.map((f) => {
            const on = sameRatio(f, canvas);
            const size = canvasSize(f, short);
            const vertical = f.height > f.width;
            const expanded = layouts === f.label;
            const why = !pictures ? "Add a video to the timeline first" : reframing ? "Already following the face" : following ? "Already following a clip. Stop it in the preview." : null;
            const which = selected > 0 ? `the ${selected} selected clip${selected === 1 ? "" : "s"}` : "every full-frame clip";
            const switching = (title: string) => (on ? title : `Switch to ${f.label}. ${title}`);
            const choose = (style: Style) => {
              closeToTrigger();
              if (style === "fill") void reframe(size.width, size.height);
              else void layOut(size.width, size.height, style);
            };
            return (
              <Fragment key={f.label}>
                <div className="group flex items-center gap-1 rounded-md pr-1 hover:bg-white/[.08] has-[:focus-visible]:bg-white/[.08]">
                  <button
                    type="button"
                    role="menuitemradio"
                    aria-checked={on}
                    onClick={(e) => {
                      chose(e.currentTarget);
                      if (!on) edit({ type: "setCanvas", ...size });
                      closeToTrigger();
                    }}
                    className="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 text-left focus-visible:outline-offset-[-2px]"
                  >
                    <span className="tabular w-11 shrink-0 text-[13px] font-medium text-fg">{f.label}</span>
                    <span className="flex-1 truncate text-[12px] text-muted">{f.hint}</span>
                    {on && <Check size={14} className="shrink-0 text-accent" />}
                  </button>
                  <span className={`shrink-0 group-hover:opacity-100 group-has-[:focus-visible]:opacity-100 ${expanded ? "opacity-100" : "opacity-0"}`}>
                    {vertical ? (
                      <Button
                        pill
                        role="menuitem"
                        aria-expanded={expanded}
                        className="h-6 px-2.5 text-[12px]"
                        title={switching(`Choose how ${which} fill the frame`)}
                        disabled={!!why}
                        disabledReason={why ?? undefined}
                        onClick={() => setLayouts(expanded ? null : f.label)}
                      >
                        Layout
                      </Button>
                    ) : (
                      <Button
                        pill
                        role="menuitem"
                        className="h-6 px-2.5 text-[12px]"
                        title={switching(`Keep the speaker's face in ${which}`)}
                        disabled={!!why}
                        disabledReason={why ?? undefined}
                        onClick={() => choose("fill")}
                      >
                        Reframe
                      </Button>
                    )}
                  </span>
                </div>
                {expanded && (
                  <div data-layouts role="group" aria-label={`Layout for ${f.label}`} className="grid grid-cols-3 gap-2 px-2 pb-2 pt-1">
                    {STYLES.map((s) => (
                      <PresetTile key={s.style} menuItem short label={s.label} selected={false} disabled={!!why} title={why ?? switching(s.title(which))} onClick={() => choose(s.style)}>
                        <span className="absolute left-1/2 top-1/2 h-6 w-[14px] -translate-x-1/2 -translate-y-1/2 overflow-hidden rounded-[2px] bg-black/40 outline outline-1 outline-white/25">
                          {s.frame}
                        </span>
                      </PresetTile>
                    ))}
                  </div>
                )}
              </Fragment>
            );
          })}
        </div>
      )}
    </div>
  );
}

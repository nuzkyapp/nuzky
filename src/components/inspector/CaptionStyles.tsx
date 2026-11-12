import { Check, Ellipsis, Plus } from "lucide-react";
import { Fragment, type CSSProperties, type ReactNode, useEffect, useRef, useState } from "react";
import { create } from "zustand";
import { api, errorText, plainError } from "../../lib/api";
import { CAPTION_STYLES, scaleStyle } from "../../lib/presets";
import type { CaptionLook } from "../../lib/speech";
import { previewHandlers, useEditor, type EditInput } from "../../lib/store";
import type { Animation, CaptionPreset } from "../../lib/types";
import { TRANSLATION_SIZE } from "../panel/CaptionLanguages";
import { Button, IconButton, Menu, lockedProps, swatchCss, useLockReason } from "../ui";

/** Caption style tiles fill the row, as many as fit. */
export const STYLE_GRID = "grid grid-cols-[repeat(auto-fill,minmax(96px,1fr))] gap-2";

/** The built-in styles for a captions track: a translation has no spoken words, so none of the styles that light or colour them. */
export const builtInStyles = (translation: boolean) => (translation ? CAPTION_STYLES.filter((s) => !s.style.highlight && !s.style.keywords) : CAPTION_STYLES);

/**
 * `preset` as it goes on a captions track: a translation's stays smaller than the speech over it and has no highlight or
 * key words; a style of the user's own (`own`) keeps its size, saved as it was.
 */
export function forTrack(preset: CaptionPreset, translation: boolean, own: boolean): CaptionPreset {
  if (!translation) return preset;
  return { ...preset, style: { ...(own ? preset.style : scaleStyle(preset.style, TRANSLATION_SIZE)), highlight: null, keywords: null } };
}

/** The user's own caption styles, one list for the Captions tab and every caption's inspector. */
const useOwnStyles = create<{ styles: CaptionPreset[]; error: string | null }>(() => ({ styles: [], error: null }));

/** The user's own caption styles, read again each time a place showing them opens, as an agent may have saved one meanwhile. */
export function useMyStyles() {
  const styles = useOwnStyles((s) => s.styles);
  const error = useOwnStyles((s) => s.error);
  useEffect(() => {
    api.captionStyles().then(
      (styles) => useOwnStyles.setState({ styles, error: null }),
      (e) => useOwnStyles.setState({ error: plainError(errorText(e)) }),
    );
  }, []);
  /** Runs a change and shows the list it returns; resolves to why it failed, or null. */
  const change = async (run: () => Promise<CaptionPreset[]>) => {
    try {
      useOwnStyles.setState({ styles: await run(), error: null });
      return null;
    } catch (e) {
      return plainError(errorText(e));
    }
  };
  return { styles, error, change };
}

/** A tile of the user's own style: its menu (…, right click, the context menu key), F2 to rename and a ref for focus. */
export type OwnTile = { menu: (at: { x: number; y: number }, keyboard: boolean) => void; rename: () => void; ref: (el: HTMLButtonElement | null) => void };

export type Tile = (preset: CaptionPreset, own?: OwnTile) => ReactNode;

/** The sentence a style tile plays, two captions timed as Whisper times speech; `key` the words Emphasis marks, a number. */
const SAMPLE = [
  { words: ["I", "made", "$500"], key: [2] },
  { words: ["in", "one", "day"], key: [] },
];
const WORD_MS = 360;
const GAP_MS = 80;
const HOLD_MS = 500;
/** At rest: the first caption fully in, its second word being said. */
const REST: SampleFrame = { loop: -1, cue: 0, lit: 1, out: false };
const reducedMotion = matchMedia("(prefers-reduced-motion: reduce)");

type SampleFrame = { loop: number; cue: number | null; lit: number | null; out: boolean };

/** A caption of the sample, held long enough after its words for a saved style's long animations to play out whole. */
const cueMs = (words: number, inMs: number, outMs: number) => words * WORD_MS + Math.max(HOLD_MS, inMs + outMs);

/** What the engine draws `t` ms into the sample: which caption, the word being said and whether it is leaving. */
function sampleAt(t: number, inMs: number, outMs: number): SampleFrame {
  const sampleMs = SAMPLE.reduce((sum, c) => sum + cueMs(c.words.length, inMs, outMs), 0) + 400;
  const loop = Math.floor(t / sampleMs);
  let at = loop * sampleMs;
  for (const [cue, c] of SAMPLE.entries()) {
    const length = cueMs(c.words.length, inMs, outMs);
    const local = t - at;
    if (local < length) {
      const word = Math.floor(local / WORD_MS);
      return { loop, cue, lit: word < c.words.length && local % WORD_MS < WORD_MS - GAP_MS ? word : null, out: local >= length - outMs };
    }
    at += length;
  }
  return { loop, cue: null, lit: null, out: false };
}

/** The words `keywords` colours, as `key_words` in the engine picks them. */
function keyWords(cue: (typeof SAMPLE)[number], keywords: CaptionPreset["style"]["keywords"]): number[] {
  const letters = (w: string) => w.replace(/[^\p{L}\p{N}]/gu, "").length;
  switch (keywords?.pick) {
    case "emphasis":
      return cue.key;
    case "first":
      return [0];
    case "last":
      return [cue.words.length - 1];
    case "longest":
      return [cue.words.reduce((best, w, i) => (letters(w) > letters(cue.words[best]) ? i : best), 0)];
    default:
      return [];
  }
}

/** Exits the engine carries on in the entry's direction, which the entry played backwards would undo. */
const EXITS = new Set(["zoomIn", "zoomOut", "slideUp", "slideDown", "slideLeft", "slideRight"]);

/** A one-shot run of the `a-*` keyframes, whose move spans 15 %–70 %, over the animation's real duration. */
function animationCss(animation: Animation | null | undefined, out: boolean): CSSProperties {
  if (!animation) return {};
  const length = animation.durationUs / 1000 / 0.55;
  const own = out && EXITS.has(animation.kind);
  const reverse = out && !own;
  return {
    animationName: own ? `a-${animation.kind}-out` : `a-${animation.kind}`,
    animationDuration: `${length}ms`,
    animationDelay: `${-(reverse ? 0.3 : 0.15) * length}ms`,
    animationDirection: reverse ? "reverse" : "normal",
    animationFillMode: "both",
    animationTimingFunction: animation.kind === "typewriter" ? "steps(5, end)" : out ? "ease-in" : "ease-out",
  };
}

/**
 * A style's captions as the engine draws them: each enters and leaves with the style's animations, karaoke lights the
 * word being said and key words take their colour. Still at rest; plays in a loop while `playing`.
 */
function CaptionSample({ preset, font, playing }: { preset: CaptionPreset; font: string | null | undefined; playing: boolean }) {
  const inMs = preset.animIn ? preset.animIn.durationUs / 1000 : 0;
  const outMs = preset.animOut ? preset.animOut.durationUs / 1000 : 0;
  const [frame, setFrame] = useState<SampleFrame | null>(null);
  useEffect(() => {
    if (!playing) return;
    const start = performance.now();
    let last = "";
    const timer = window.setInterval(() => {
      const next = sampleAt(performance.now() - start, inMs, outMs);
      const id = `${next.loop} ${next.cue} ${next.lit} ${next.out}`;
      if (id !== last) {
        last = id;
        setFrame(next);
      }
    }, 30);
    return () => {
      window.clearInterval(timer);
      setFrame(null);
    };
  }, [playing, inMs, outMs]);
  const at = playing ? (frame ?? sampleAt(0, inMs, outMs)) : REST;
  if (at.cue === null) return null;
  const cue = SAMPLE[at.cue];
  const style = { ...preset.style, fontFamily: preset.style.fontFamily ?? font };
  const keys = keyWords(cue, style.keywords);
  return (
    <span
      key={`${at.loop} ${at.cue} ${at.out}`}
      data-caption-sample
      className="inline-block max-w-full overflow-hidden whitespace-nowrap rounded px-1 leading-6"
      style={{ ...swatchCss(style, 12), ...(at.loop < 0 ? {} : animationCss(at.out ? preset.animOut : preset.animIn, at.out)) }}
    >
      {cue.words.map((word, i) => {
        const color = style.highlight && i === at.lit ? style.highlight : keys.includes(i) ? style.keywords?.color : undefined;
        return (
          <Fragment key={i}>
            {i > 0 && " "}
            {color ? <span style={{ color }}>{word}</span> : word}
          </Fragment>
        );
      })}
    </span>
  );
}

/** A caption style tile: its captions playing in the style over its name, checked while the captions have the look. */
export function StyleTile({
  preset,
  font,
  current,
  disabled,
  title,
  onApply,
  preview,
  own,
}: {
  preset: CaptionPreset;
  /** The captions' font, which a style without its own keeps. */
  font: string | null | undefined;
  current: boolean;
  disabled: boolean;
  title: string;
  onApply: () => void;
  /** The change the preview draws while the tile is pointed at or focused, before a click applies it. */
  preview?: EditInput | null;
  own?: OwnTile;
}) {
  const lock = useLockReason();
  const [playing, setPlaying] = useState(false);
  // The user's own styles are not the project: recognition running leaves renaming and deleting them alone.
  const openMenu = (at: { x: number; y: number }, keyboard: boolean) => !lock && own?.menu(at, keyboard);
  const drawn = previewHandlers(disabled ? null : preview);
  const play = (on: boolean) => setPlaying(on && !reducedMotion.matches);
  return (
    <div className="group relative min-w-0">
      {/* Beside the button, under its clear box, so the button's text stays the style's name. */}
      <span
        aria-hidden
        className={`pointer-events-none absolute inset-x-0 top-0 flex h-16 items-center justify-center overflow-hidden rounded-[10px] bg-white/[.16] px-1 ${disabled ? "opacity-40" : ""}`}
      >
        <CaptionSample preset={preset} font={font} playing={playing} />
      </span>
      <button
        type="button"
        ref={own?.ref}
        data-style-tile
        aria-pressed={current}
        aria-disabled={disabled || undefined}
        title={title}
        onClick={disabled ? undefined : onApply}
        onClickCapture={drawn.onClickCapture}
        onPointerEnter={() => {
          play(true);
          drawn.onPointerEnter?.();
        }}
        onPointerLeave={() => {
          play(false);
          drawn.onPointerLeave();
        }}
        onFocus={(e) => {
          if (e.currentTarget.matches(":focus-visible")) play(true);
          drawn.onFocus?.();
        }}
        onBlur={() => {
          play(false);
          drawn.onBlur();
        }}
        onContextMenu={(e) => {
          if (!own) return;
          e.preventDefault();
          openMenu({ x: e.clientX, y: e.clientY }, false);
        }}
        onKeyDown={(e) => {
          if (!own || lock) return;
          if (e.key === "F2") {
            e.preventDefault();
            own.rename();
          } else if (e.key === "ContextMenu" || (e.shiftKey && e.key === "F10")) {
            e.preventDefault();
            const r = e.currentTarget.getBoundingClientRect();
            openMenu({ x: r.left, y: r.bottom + 4 }, true);
          }
        }}
        className="flex w-full min-w-0 flex-col gap-1.5 rounded-[10px] text-left aria-disabled:cursor-not-allowed"
      >
        <span
          className={`relative block h-16 w-full rounded-[10px] border transition-colors duration-[120ms] ease-out ${
            current ? "border-accent shadow-[0_0_0_1px_var(--color-accent)]" : disabled ? "border-white/[.08]" : "border-white/[.08] group-hover:border-white/30"
          }`}
        >
          {current && (
            <span aria-hidden className="absolute top-1 right-1 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-accent-strong text-white shadow-[0_1px_2px_rgb(0_0_0/.5)]">
              <Check size={10} strokeWidth={3} />
            </span>
          )}
        </span>
        <span className={`w-full truncate text-center text-[11px] ${current ? "font-medium text-fg" : "text-muted"} ${disabled ? "opacity-40" : ""}`}>{preset.name}</span>
      </button>
      {own && !lock && (
        <button
          type="button"
          tabIndex={-1}
          aria-hidden
          title="More"
          onClick={(e) => {
            const r = e.currentTarget.getBoundingClientRect();
            openMenu({ x: r.left, y: r.bottom + 4 }, false);
          }}
          className="absolute top-10 right-1 flex h-5 w-5 items-center justify-center rounded-full bg-black/55 text-fg opacity-0 transition-opacity duration-[120ms] group-hover:opacity-100 hover:bg-black/75"
        >
          <Ellipsis size={12} />
        </button>
      )}
    </div>
  );
}

/** The user's own styles: save the current look under a name, apply, rename and delete. */
export function MyStyles({ look, mine, tile, grid = STYLE_GRID }: { look: CaptionLook; mine: ReturnType<typeof useMyStyles>; tile: Tile; grid?: string }) {
  const lock = useLockReason();
  const [naming, setNaming] = useState<{ from: string | null; error?: string } | null>(null);
  const [menu, setMenu] = useState<{ name: string; at: { x: number; y: number }; keyboard: boolean } | null>(null);
  const add = useRef<HTMLButtonElement>(null);
  const tiles = useRef(new Map<string, HTMLButtonElement>());
  const [focus, setFocus] = useState<{ name: string | null } | null>(null);
  useEffect(() => {
    if (!focus) return;
    ((focus.name && tiles.current.get(focus.name)) || add.current)?.focus();
    setFocus(null);
  }, [focus]);
  const unique = (base: string) => {
    for (let n = 1; ; n++) if (!mine.styles.some((s) => s.name.toLowerCase() === `${base} ${n}`.toLowerCase())) return `${base} ${n}`;
  };
  const done = (name: string | null) => {
    setNaming(null);
    setFocus({ name });
  };
  const save = async (name: string) => {
    const from = naming?.from ?? null;
    if (lock || from === name) return done(from);
    const error = from ? await mine.change(() => api.renameCaptionStyle(from, name)) : await mine.change(() => api.saveCaptionStyle({ name, style: look.style, animIn: look.animIn, animOut: look.animOut }));
    if (error) setNaming({ from, error });
    else done(name);
  };
  const remove = async (name: string) => {
    const at = mine.styles.findIndex((s) => s.name === name);
    const style = mine.styles[at];
    if (!style) return;
    const error = await mine.change(() => api.deleteCaptionStyle(name));
    if (error) return useEditor.getState().toast({ kind: "error", text: error });
    const rest = mine.styles.filter((s) => s.name !== name);
    setFocus({ name: (rest[at] ?? rest[at - 1])?.name ?? null });
    useEditor.getState().toast({ kind: "info", text: `Deleted “${name}”`, action: { label: "Undo", run: () => void undo(style) } });
  };
  const undo = async (style: CaptionPreset) => {
    const error = await mine.change(() => api.saveCaptionStyle(style));
    if (error) useEditor.getState().toast({ kind: "error", text: error });
  };

  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex min-h-6 items-center justify-between gap-2">
        <span className="text-[12px] text-muted">My styles</span>
        <IconButton ref={add} label="Save the current look as a style" disabled={!!naming} onClick={() => setNaming({ from: null })} className="h-6 w-6">
          <Plus size={14} />
        </IconButton>
      </div>
      {naming && <NameField key={naming.from ?? ""} initial={naming.from ?? unique("My style")} error={naming.error} onSave={save} onCancel={() => done(naming.from)} />}
      {mine.error && <p className="text-[12px] text-danger">{mine.error}</p>}
      {mine.styles.length > 0 ? (
        <div className={grid}>
          {mine.styles.map((s) =>
            tile(s, {
              menu: (at, keyboard) => setMenu({ name: s.name, at, keyboard }),
              rename: () => setNaming({ from: s.name }),
              ref: (el) => void (el ? tiles.current.set(s.name, el) : tiles.current.delete(s.name)),
            }),
          )}
        </div>
      ) : (
        !naming &&
        !mine.error && (
          <div className={grid}>
            <button
              type="button"
              {...lockedProps(lock)}
              onClick={lock ? undefined : () => setNaming({ from: null })}
              className="flex h-16 min-w-0 items-center justify-center gap-1 whitespace-nowrap rounded-[10px] border border-dashed border-white/[.15] text-[11px] text-muted transition-colors duration-[120ms] hover:border-white/30 hover:text-fg aria-disabled:cursor-not-allowed aria-disabled:opacity-40"
            >
              <Plus size={12} /> Save look
            </button>
          </div>
        )
      )}
      {menu && (
        <Menu
          label={`${menu.name} style`}
          at={menu.at}
          keyboard={menu.keyboard}
          onClose={(chose) => {
            if (!chose) setFocus({ name: menu.name });
            setMenu(null);
          }}
          items={[
            { label: "Rename", shortcut: "F2", run: () => setNaming({ from: menu.name }) },
            { label: "Delete", danger: true, run: () => void remove(menu.name) },
          ]}
        />
      )}
    </div>
  );
}

function NameField({ initial, error, onSave, onCancel }: { initial: string; error?: string; onSave: (name: string) => void; onCancel: () => void }) {
  const [name, setName] = useState(initial);
  const ref = useRef<HTMLInputElement>(null);
  useEffect(() => ref.current?.select(), []);
  useEffect(() => {
    if (error) ref.current?.focus();
  }, [error]);
  const submit = () => (name.trim() ? onSave(name.trim()) : onCancel());
  return (
    <div className="flex flex-col gap-1">
      <div className="flex items-center gap-1.5">
        <input
          ref={ref}
          aria-label="Style name"
          aria-invalid={!!error || undefined}
          value={name}
          maxLength={40}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            e.stopPropagation();
            if (e.key === "Enter") submit();
            else if (e.key === "Escape") onCancel();
          }}
          className={`h-8 min-w-0 flex-1 rounded-lg border bg-white/[.055] px-2.5 text-[13px] text-fg outline-none ${error ? "border-danger" : "border-white/[.08] focus:border-accent"}`}
        />
        <Button onClick={submit}>Save</Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
      {error && <p className="text-[12px] text-danger">{error}</p>}
    </div>
  );
}

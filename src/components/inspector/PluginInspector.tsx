import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type MouseEvent, type RefObject } from "react";
import { Eye, MoreHorizontal, Sparkles, X } from "lucide-react";
import { describePeek } from "../../lib/peek";
import { fixable, fixWithAi, own, plainDetail, pluginBlocked, previewPlugin, runPlugin, setAuto, setValue, shownInputs, usePlugins, valuesFor, type Plugin, type PluginInput, type PluginRow, type PluginValue, type Trigger } from "../../lib/plugins";
import { useSpeech, useTranscriptView } from "../../lib/speech";
import { useEditor } from "../../lib/store";
import { Select } from "../panel/SpeechControls";
import { PluginMark, pluginMenu } from "../panel/PluginsTab";
import { AiLock, Button, Checkbox, ColorInput, IconButton, LockReason, Menu, Section, Segmented, Slider, Switch, useLockReason } from "../ui";

/** How long the settings rest before the plugin runs again without changing anything, so dragging a slider does not run it per pixel. */
const PREVIEW_DELAY_MS = 300;

const MAC = navigator.userAgent.includes("Mac");

const LOOKING = <span className="text-[12px] text-muted">Looking…</span>;

/** `rows`: what a dynamic list offers, undefined while the plugin is still looking. */
export function Control({ input, value, rows, onChange }: { input: PluginInput; value: PluginValue; rows?: PluginRow[]; onChange: (v: PluginValue) => void }) {
  const lock = useLockReason();
  if (input.type === "choice" && input.options === "dynamic")
    return (
      <div className="flex flex-col gap-1.5">
        <span className="text-[12px] text-muted">{input.label}</span>
        {rows === undefined ? (
          LOOKING
        ) : (
          <Select aria-label={input.label} disabled={!!lock || !rows.length} title={lock ?? (rows.length ? undefined : "None found.")} value={value as string} onChange={(e) => onChange(e.target.value)}>
            <option value="">{rows.length ? "Choose…" : "None found."}</option>
            {rows.map((r) => (
              <option key={r.value} value={r.value}>
                {r.label}
              </option>
            ))}
          </Select>
        )}
      </div>
    );
  switch (input.type) {
    case "number":
      return <Slider label={input.label} value={value as number} min={input.min} max={input.max} step={input.step} unit={input.unit} onChange={onChange} />;
    case "toggle":
      return (
        <div className="flex items-center justify-between gap-2">
          <span className="text-[12px] text-muted">{input.label}</span>
          <Switch label={input.label} checked={value as boolean} onChange={onChange} />
        </div>
      );
    case "choice": {
      const options = input.options as PluginRow[];
      return (
        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] text-muted">{input.label}</span>
          {options.length <= 4 ? (
            <Segmented label={input.label} options={options.map((o) => ({ id: o.value, label: o.label }))} value={value as string} onChange={onChange} />
          ) : (
            <Select aria-label={input.label} disabled={!!lock} title={lock ?? undefined} value={value as string} onChange={(e) => onChange(e.target.value)}>
              {options.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </Select>
          )}
        </div>
      );
    }
    case "multi": {
      const options = input.options === "dynamic" ? rows : input.options;
      const checked = value as string[];
      return (
        <div role="group" aria-label={input.label} className="flex flex-col gap-2">
          <span className="text-[12px] text-muted">{input.label}</span>
          {options === undefined && LOOKING}
          {options?.length === 0 && <span className="text-[12px] text-muted">None found.</span>}
          {!!options?.length && (
            <div className="-mx-1 flex max-h-[184px] flex-col gap-2 overflow-y-auto px-1 py-0.5">
              {options.map((o) => (
                <Checkbox key={o.value} label={o.label} checked={checked.includes(o.value)} onChange={(on) => onChange(on ? [...checked, o.value] : checked.filter((v) => v !== o.value))} />
              ))}
            </div>
          )}
        </div>
      );
    }
    case "text":
      return (
        <label className="flex flex-col gap-1.5">
          <span className="text-[12px] text-muted">{input.label}</span>
          <input
            type="text"
            maxLength={500}
            disabled={!!lock}
            title={lock ?? undefined}
            value={value as string}
            onChange={(e) => onChange(e.target.value)}
            className="h-8 rounded-lg bg-white/[.09] px-2.5 text-[13px] text-fg shadow-[inset_0_0_0_1px_rgb(255_255_255/.06),inset_0_1px_0_rgb(255_255_255/.06)] disabled:opacity-40"
          />
        </label>
      );
    case "color":
      return <ColorInput label={input.label} value={value as string} onChange={onChange} />;
  }
}

/**
 * What running it would do, from the preview, or why it cannot run. While a newer preview is on its way the last one
 * stays, `stale`, so the panel does not jump on every change.
 */
function useOutcome(p: Plugin, key: string, running: boolean) {
  const preview = usePlugins((s) => s.preview);
  const snap = useEditor((s) => s.snap);
  const { view } = useTranscriptView();
  const blocked = pluginBlocked(p, !!view?.words.length);
  // Undo's words do not name every change (a background, a thumbnail), so "nothing" means the same project.
  const { lines, marked } = useMemo(() => {
    if (!preview?.project || !snap) return { lines: null, marked: false };
    if (JSON.stringify(preview.project) === JSON.stringify(snap.project)) return { lines: [], marked: false };
    const said = describePeek(snap.project, preview.project, true);
    // The timeline marks only clips it already shows, never ones the plugin would add.
    const shown = new Set(snap.project.tracks.flatMap((t) => t.clips.map((c) => c.id)));
    return { lines: said.lines.length ? said.lines : ["Changes the project"], marked: [...said.marks.keys()].some((id) => shown.has(id)) };
  }, [preview, snap]);
  if (blocked) return { blocked, lines: null, marked, stale: false, waiting: false, cannotRun: true };
  if (!preview) return { blocked: null, lines: null, marked, stale: true, waiting: false };
  const stale = running || preview.key !== key || preview.revision !== snap?.revision;
  if (preview.error) return { blocked: preview.error, lines: null, marked, stale, waiting: preview.waiting };
  if (lines && lines.length === 0) return { blocked: "Nothing left to change.", lines: null, marked, stale, waiting: false };
  return { blocked: null, lines, marked, stale, waiting: false };
}

function PluginBody({ plugin: p, primary }: { plugin: Plugin; primary: RefObject<(() => void) | null> }) {
  const stored = usePlugins((s) => own(s.values, p.id));
  // By content, so a preview that finds the same rows again does not start another one.
  const rowsJson = usePlugins((s) => JSON.stringify(own(s.rows, p.id) ?? null));
  const rows = useMemo(() => JSON.parse(rowsJson) as Record<string, PluginRow[]> | null, [rowsJson]);
  const all = useMemo(() => valuesFor(p, stored, rows ?? undefined), [p, stored, rows]);
  const revision = useEditor((s) => s.snap?.revision);
  const selection = useEditor((s) => s.selection);
  const running = usePlugins((s) => s.running === p.id);
  const pressed = usePlugins((s) => (s.running === p.id ? s.pressed : null));
  const lock = useLockReason();
  // The playhead counts once it rests: a plugin may work where it is, but playback must not run it every frame.
  const playhead = useEditor((s) => (s.playing ? null : s.timeUs));
  const key = JSON.stringify([p.id, all, selection, playhead]);
  const outcome = useOutcome(p, key, running);
  // Prepared sound and new words change what a plugin that needs them finds.
  const heard = useSpeech((s) => (p.needs.length ? s.stored : 0));

  useEffect(() => {
    if (p.error || running) return;
    const timer = window.setTimeout(() => void previewPlugin(p, all, key), PREVIEW_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [p, all, key, revision, running, heard]);

  const [first, ...more] = p.actions;
  const primaryOff = (!!outcome.blocked && !outcome.stale && !outcome.waiting) || running || !!lock;
  // The preview is of the primary action, so only what stops every run turns the others off.
  const moreReason = running ? "Running…" : outcome.cannotRun ? (outcome.blocked ?? undefined) : undefined;
  useEffect(() => {
    primary.current = primaryOff ? null : () => void runPlugin(p, first.id);
    return () => {
      primary.current = null;
    };
  }, [primary, primaryOff, p, first.id]);

  const shown = shownInputs(p, all);
  const placed = new Set(p.sections.flatMap((s) => s.inputs));
  const groups = [{ label: undefined as string | undefined, ids: p.inputs.filter((i) => !placed.has(i.id)).map((i) => i.id) }, ...p.sections.map((s) => ({ label: s.label, ids: s.inputs }))]
    .map((g) => ({ label: g.label, inputs: p.inputs.filter((i) => g.ids.includes(i.id) && shown.has(i.id)) }))
    .filter((g) => g.inputs.length > 0);

  const said = outcome.lines && `${outcome.lines.slice(0, 3).join(". ")}.`;
  return (
    <>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {groups.map((g, i) => (
          <Section key={i} title={g.label}>
            {g.inputs.map((input) => (
              <Control key={input.id} input={input} value={all[input.id]} rows={rows ? (own(rows, input.id) ?? []) : undefined} onChange={(v) => setValue(p.id, input.id, v)} />
            ))}
          </Section>
        ))}
        <LockReason.Provider value={null}>
          <AutoSwitches plugin={p} />
        </LockReason.Provider>
      </div>
      {/* Pinned under the settings, so the buttons are in view however many there are. */}
      <div className="flex shrink-0 flex-col gap-2.5 border-t border-white/[.07] px-4 py-3.5">
        <div className={`text-[12px] transition-opacity duration-[120ms] ${outcome.stale ? "opacity-50" : ""}`}>
          {outcome.blocked ? (
            <p className="text-muted">{outcome.blocked}</p>
          ) : said ? (
            <div className="flex gap-2 text-fg/85" title={outcome.marked ? `${said} Marked on the timeline.` : said}>
              <Eye size={14} className="mt-px shrink-0 text-muted" aria-hidden />
              <span className="line-clamp-2">
                {said}
                {outcome.marked && <span className="text-muted"> Marked on the timeline.</span>}
              </span>
            </div>
          ) : (
            <p className="text-muted">{running ? "\u00a0" : "Checking…"}</p>
          )}
        </div>
        <Button
          variant="primary"
          pill
          className="h-8 w-full"
          data-plugin-action={first.id}
          title={`${first.label} (${MAC ? "Cmd" : "Ctrl"}+Enter)`}
          aria-keyshortcuts={MAC ? "Meta+Enter" : "Control+Enter"}
          disabled={primaryOff}
          disabledReason={running ? "Running…" : (outcome.blocked ?? undefined)}
          onClick={() => void runPlugin(p, first.id)}
        >
          {pressed === first.id ? "Running…" : first.label}
        </Button>
        {more.length > 0 && (
          <div className="flex flex-wrap gap-2">
            {more.map((a) => (
              <Button
                key={a.id}
                pill
                className="h-8 min-w-0 grow-0 basis-[calc(50%-4px)] px-3"
                data-plugin-action={a.id}
                title={a.label}
                disabled={!!moreReason}
                disabledReason={moreReason}
                onClick={() => void runPlugin(p, a.id)}
              >
                <span className="truncate">{pressed === a.id ? "Running…" : a.label}</span>
              </Button>
            ))}
          </div>
        )}
      </div>
    </>
  );
}

function AutoSwitch({ plugin: p, trigger, label }: { plugin: Plugin; trigger: Trigger; label: string }) {
  const on = usePlugins((s) => s.auto[trigger].includes(p.id));
  return (
    <div className="flex items-center justify-between gap-2">
      <span className="text-[12px] text-muted">{label}</span>
      <Switch label={label} checked={on} onChange={(v) => setAuto(p.id, trigger, v)} />
    </div>
  );
}

/** Nuzky's own settings, not an edit of the project, so the AI lock leaves them free: a stuck plugin can be turned off. */
function AutoSwitches({ plugin: p }: { plugin: Plugin }) {
  const on = usePlugins((s) => s.auto.import.includes(p.id) || s.auto.export.includes(p.id));
  return (
    <section className="flex flex-col gap-2 border-b border-white/[.07] px-4 py-3 last:border-b-0">
      <AutoSwitch plugin={p} trigger="import" label="Run after import" />
      <AutoSwitch plugin={p} trigger="export" label="Run before export" />
      {on && <p className="text-[12px] text-muted">{p.inputs.length ? "Runs on the whole project with these settings." : "Runs on the whole project."}</p>}
    </section>
  );
}

/** The chosen plugin's settings, what it would change, and its button. Closing it brings back what the inspector showed. */
export function PluginInspector({ plugin: p }: { plugin: Plugin }) {
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  const primary = useRef<(() => void) | null>(null);
  const showMenu = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
  };
  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.key !== "Enter" || !(e.metaKey || e.ctrlKey)) return;
    e.preventDefault();
    primary.current?.();
  };
  return (
    <div className="flex min-h-0 flex-1 flex-col" onKeyDown={onKeyDown}>
      <div className="flex shrink-0 items-start gap-2.5 px-4 pt-4 pb-3">
        <PluginMark plugin={p} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] leading-5 font-semibold tracking-[-0.015em] text-fg">{p.name}</div>
          {p.description && (
            <div className="mt-[3px] line-clamp-2 text-[12px] text-muted" title={p.description}>
              {p.description}
            </div>
          )}
        </div>
        <IconButton label={`More for ${p.name}`} className="size-7 shrink-0" onClick={showMenu}>
          <MoreHorizontal size={16} />
        </IconButton>
        <IconButton label="Close plugin" className="size-7 shrink-0" onClick={() => usePlugins.setState({ open: null, preview: null })}>
          <X size={16} />
        </IconButton>
        {menu && <Menu items={pluginMenu(p)} at={{ x: menu.x, y: menu.y, align: "end" }} label={p.name} keyboard={menu.keyboard} onClose={() => setMenu(null)} />}
      </div>
      <div className="flex min-h-0 flex-1 flex-col border-t border-white/[.07]">
        {p.error ? (
          <div className="min-h-0 flex-1 overflow-y-auto">
            <AutoSwitches plugin={p} />
            <section className="flex flex-col items-start gap-3 px-4 py-4">
              <p className="text-[12px] text-danger">Can’t load: {plainDetail(p.error)}</p>
              {fixable(p.error) && (
                <Button className="h-8 gap-1.5 px-3" onClick={() => fixWithAi(p, p.error ?? "")}>
                  <Sparkles size={14} className="text-accent" /> Fix with AI
                </Button>
              )}
            </section>
          </div>
        ) : !p.runs ? (
          <div className="min-h-0 flex-1 overflow-y-auto">
            <section className="flex flex-col gap-2.5 px-4 py-4">
              {p.effects.map((e) => (
                <div key={e.id} className="flex flex-col gap-1">
                  <span className="text-[13px] font-medium text-fg">{e.name}</span>
                  {e.error && <p className="text-[12px] text-danger">Can’t draw: {plainDetail(e.error)}</p>}
                </div>
              ))}
              <p className="text-[12px] text-muted">{pluginBlocked(p, false)}</p>
            </section>
          </div>
        ) : (
          <AiLock>
            <PluginBody key={p.id} plugin={p} primary={primary} />
          </AiLock>
        )}
      </div>
    </div>
  );
}

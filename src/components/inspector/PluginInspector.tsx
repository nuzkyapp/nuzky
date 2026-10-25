import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { Eye, MoreHorizontal, Sparkles, X } from "lucide-react";
import { describePeek } from "../../lib/peek";
import { fixable, fixWithAi, plainDetail, pluginBlocked, previewPlugin, runPlugin, setAuto, setValue, usePlugins, valuesFor, type Plugin, type PluginInput, type PluginValue, type Trigger } from "../../lib/plugins";
import { useSpeech, useTranscriptView } from "../../lib/speech";
import { useEditor } from "../../lib/store";
import { Select } from "../panel/SpeechControls";
import { PluginMark, pluginMenu } from "../panel/PluginsTab";
import { AiLock, Button, ColorInput, IconButton, Menu, Section, Segmented, Slider, Switch, useLockReason } from "../ui";

/** How long the settings rest before the plugin runs again without changing anything, so dragging a slider does not run it per pixel. */
const PREVIEW_DELAY_MS = 300;

function Control({ input, value, onChange }: { input: PluginInput; value: PluginValue; onChange: (v: PluginValue) => void }) {
  const lock = useLockReason();
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
    case "choice":
      return (
        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] text-muted">{input.label}</span>
          {input.options.length <= 4 ? (
            <Segmented label={input.label} options={input.options.map((o) => ({ id: o.value, label: o.label }))} value={value as string} onChange={onChange} />
          ) : (
            <Select aria-label={input.label} disabled={!!lock} value={value as string} onChange={(e) => onChange(e.target.value)}>
              {input.options.map((o) => (
                <option key={o.value} value={o.value}>
                  {o.label}
                </option>
              ))}
            </Select>
          )}
        </div>
      );
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
function useOutcome(p: Plugin, key: string) {
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
  if (blocked) return { blocked, lines: null, marked, stale: false, waiting: false };
  if (!preview) return { blocked: null, lines: null, marked, stale: true, waiting: false };
  const stale = preview.key !== key || preview.revision !== snap?.revision;
  if (preview.error) return { blocked: preview.error, lines: null, marked, stale, waiting: preview.waiting };
  if (lines && lines.length === 0) return { blocked: "Nothing left to change.", lines: null, marked, stale, waiting: false };
  return { blocked: null, lines, marked, stale, waiting: false };
}

function PluginBody({ plugin: p }: { plugin: Plugin }) {
  const stored = usePlugins((s) => s.values[p.id]);
  const all = useMemo(() => valuesFor(p, stored), [p, stored]);
  const revision = useEditor((s) => s.snap?.revision);
  const selection = useEditor((s) => s.selection);
  const running = usePlugins((s) => s.running === p.id);
  // The playhead counts once it rests: a plugin may work where it is, but playback must not run it every frame.
  const playhead = useEditor((s) => (s.playing ? null : s.timeUs));
  const key = JSON.stringify([p.id, all, selection, playhead]);
  const outcome = useOutcome(p, key);
  // Prepared sound and new words change what a plugin that needs them finds.
  const heard = useSpeech((s) => (p.needs.length ? s.stored : 0));

  useEffect(() => {
    if (p.error || running) return;
    const timer = window.setTimeout(() => void previewPlugin(p, all, key), PREVIEW_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [p, all, key, revision, running, heard]);

  return (
    <>
      {p.inputs.length > 0 && (
        <Section>
          {p.inputs.map((input) => (
            <Control key={input.id} input={input} value={all[input.id]} onChange={(v) => setValue(p.id, input.id, v)} />
          ))}
        </Section>
      )}
      <section className="flex flex-col gap-2.5 px-4 py-4">
        {outcome.lines && (
          <div className={`flex gap-2 text-[12px] text-fg/85 transition-opacity duration-[120ms] ${outcome.stale ? "opacity-50" : ""}`}>
            <Eye size={14} className="mt-px shrink-0 text-muted" aria-hidden />
            <span>
              {outcome.lines.slice(0, 3).join(". ")}.{outcome.marked && <span className="text-muted"> Marked on the timeline.</span>}
            </span>
          </div>
        )}
        <Button
          variant="primary"
          pill
          className="h-8 w-full"
          disabled={(!!outcome.blocked && !outcome.stale && !outcome.waiting) || running}
          disabledReason={running ? "Running…" : (outcome.blocked ?? undefined)}
          onClick={() => void runPlugin(p)}
        >
          {running ? "Running…" : p.action}
        </Button>
        {outcome.blocked && <p className={`text-[12px] text-muted ${outcome.stale ? "opacity-50" : ""}`}>{outcome.blocked}</p>}
      </section>
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
    <section className="flex flex-col gap-2 border-b border-white/[.07] px-4 py-3">
      <AutoSwitch plugin={p} trigger="import" label="Run after import" />
      <AutoSwitch plugin={p} trigger="export" label="Run before export" />
      {on && <p className="text-[12px] text-muted">{p.inputs.length ? "Runs on the whole project with these settings." : "Runs on the whole project."}</p>}
    </section>
  );
}

/** The chosen plugin's settings, what it would change, and its button. Closing it brings back what the inspector showed. */
export function PluginInspector({ plugin: p }: { plugin: Plugin }) {
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  const showMenu = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
  };
  return (
    <>
      <div className="flex shrink-0 items-start gap-2.5 px-4 pt-4 pb-3">
        <PluginMark plugin={p} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] leading-5 font-semibold tracking-[-0.015em] text-fg">{p.name}</div>
          {p.description && <div className="mt-[3px] text-[12px] text-muted">{p.description}</div>}
        </div>
        <IconButton label={`More for ${p.name}`} className="size-7 shrink-0" onClick={showMenu}>
          <MoreHorizontal size={16} />
        </IconButton>
        <IconButton label="Close plugin" className="size-7 shrink-0" onClick={() => usePlugins.setState({ open: null, preview: null })}>
          <X size={16} />
        </IconButton>
        {menu && <Menu items={pluginMenu(p)} at={{ x: menu.x, y: menu.y, align: "end" }} label={p.name} keyboard={menu.keyboard} onClose={() => setMenu(null)} />}
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto border-t border-white/[.07]">
        <AutoSwitches plugin={p} />
        {p.error ? (
          <section className="flex flex-col items-start gap-3 px-4 py-4">
            <p className="text-[12px] text-danger">Can’t load: {plainDetail(p.error)}</p>
            {fixable(p.error) && (
              <Button className="h-8 gap-1.5 px-3" onClick={() => fixWithAi(p, p.error ?? "")}>
                <Sparkles size={14} className="text-accent" /> Fix with AI
              </Button>
            )}
          </section>
        ) : (
          <AiLock>
            <PluginBody key={p.id} plugin={p} />
          </AiLock>
        )}
      </div>
    </>
  );
}

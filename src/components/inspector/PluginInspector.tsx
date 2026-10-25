import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { Eye, MoreHorizontal, X } from "lucide-react";
import { describePeek } from "../../lib/peek";
import { defaults, pluginBlocked, previewPlugin, runPlugin, setValue, usePlugins, type Plugin, type PluginInput, type PluginValue } from "../../lib/plugins";
import { useTranscriptView } from "../../lib/speech";
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

/** What running it would do, from the preview, or why it cannot run. */
function useOutcome(p: Plugin, key: string) {
  const preview = usePlugins((s) => s.preview);
  const snap = useEditor((s) => s.snap);
  const { view } = useTranscriptView();
  const blocked = pluginBlocked(p, !!view?.words.length);
  const lines = useMemo(() => (preview?.project && snap ? describePeek(snap.project, preview.project).lines : null), [preview, snap]);
  if (blocked) return { blocked, lines: null };
  if (!preview || preview.key !== key || preview.revision !== snap?.revision) return { blocked: null, lines: null, checking: true };
  if (preview.error) return { blocked: preview.error, lines: null };
  if (lines && lines.length === 0) return { blocked: "Changes nothing in this project now.", lines: null };
  return { blocked: null, lines };
}

function PluginBody({ plugin: p }: { plugin: Plugin }) {
  const stored = usePlugins((s) => s.values[p.id]);
  const all = useMemo(() => ({ ...defaults(p), ...stored }), [p, stored]);
  const revision = useEditor((s) => s.snap?.revision);
  const selection = useEditor((s) => s.selection);
  const running = usePlugins((s) => s.running === p.id);
  const key = JSON.stringify([p.id, all, selection]);
  const outcome = useOutcome(p, key);

  // Runs it without changing anything once the settings, the selection or the project rest.
  useEffect(() => {
    if (p.error || running) return;
    const timer = window.setTimeout(() => void previewPlugin(p, all, key), PREVIEW_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [p, all, key, revision, running]);

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
          <div className="flex gap-2 text-[12px] text-fg/85">
            <Eye size={14} className="mt-px shrink-0 text-muted" aria-hidden />
            <span>
              {outcome.lines.slice(0, 3).join(". ")}. <span className="text-muted">Marked on the timeline.</span>
            </span>
          </div>
        )}
        <Button variant="primary" pill className="h-8 w-full" disabled={!!outcome.blocked || running || !!outcome.checking} disabledReason={outcome.blocked ?? (running ? "Running…" : "Checking what it would change…")} onClick={() => void runPlugin(p)}>
          {running ? "Running…" : p.action}
        </Button>
        {outcome.blocked && <p className="text-[12px] text-muted">{outcome.blocked}</p>}
      </section>
    </>
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
        <PluginMark plugin={p} size={28} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] leading-5 font-semibold tracking-[-0.015em] text-fg">{p.name}</div>
          <div className="mt-[3px] text-[12px] text-muted">{p.description}</div>
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
        <AiLock>
          <PluginBody key={p.id} plugin={p} />
        </AiLock>
      </div>
    </>
  );
}

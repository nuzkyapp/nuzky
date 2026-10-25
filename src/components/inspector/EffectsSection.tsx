import { useEffect, useState, type MouseEvent } from "react";
import { AlertTriangle, Plus, X } from "lucide-react";
import { installedEffects, loadPlugins, plainDetail, useEffectIssues, usePlugins, type EffectParam, type PluginEffect, type PluginValue } from "../../lib/plugins";
import { editClip } from "../../lib/store";
import type { Clip, Effect } from "../../lib/types";
import { IconButton, Menu, Section } from "../ui";
import { QUIET } from "./Header";
import { Control } from "./PluginInspector";

/** As `MAX_CLIP_EFFECTS` in the engine. */
const MAX_EFFECTS = 4;

/** The value a control shows: the clip's, within the installed bounds, else the default. */
function shown(param: EffectParam, effect: Effect): PluginValue {
  const v = effect.params[param.id];
  if (param.type === "number") return typeof v === "number" ? Math.min(param.max, Math.max(param.min, v)) : param.default;
  return typeof v === typeof param.default ? (v as PluginValue) : param.default;
}

const sameEffect = (a: { plugin: string; effect: string }, b: PluginEffect) => a.plugin === b.plugin && a.effect === b.id;

/** A video or image clip's plugin effects, drawn over its picture in order, with their settings. */
export function EffectsSection({ clip }: { clip: Clip }) {
  const list = usePlugins((s) => s.list);
  const issues = useEffectIssues((s) => s.issues);
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  useEffect(() => {
    if (!list) void loadPlugins();
  }, [list]);
  const effects = clip.effects ?? [];
  const installed = installedEffects(list);
  // Built from the latest confirmed effects, so a second slider never overwrites the first.
  const set = (build: (effects: Effect[]) => Effect[], coalesce?: string) =>
    editClip(clip.id, (c) => ({ type: "setEffects", clipId: c.id, effects: build(c.effects ?? []) }), coalesce);
  const add = (e: PluginEffect) =>
    set((all) => [...all, { plugin: e.plugin, effect: e.id, name: e.name, version: e.version, params: Object.fromEntries(e.params.map((p) => [p.id, p.default])) }]);
  const blocked =
    effects.length >= MAX_EFFECTS ? `a clip has at most ${MAX_EFFECTS} effects` : installed.length === 0 ? "no plugin adds one yet; ask the AI to make one, such as a VHS look" : null;
  const showMenu = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
  };
  const named = (e: PluginEffect) => (installed.filter((o) => o.name === e.name).length > 1 ? `${e.name} · ${e.pluginName}` : e.name);
  return (
    <Section
      title="Effects"
      actions={
        <IconButton label={blocked ? `Add effect: ${blocked}` : "Add effect"} className={QUIET} disabled={!!blocked} onClick={showMenu}>
          <Plus size={14} />
        </IconButton>
      }
    >
      {effects.map((effect, index) => {
        const def = installed.find((e) => sameEffect(effect, e));
        const issue = issues.find((i) => i.plugin === effect.plugin && i.effect === effect.effect && i.clipIds.includes(clip.id));
        return (
          <div key={`${index}:${effect.plugin}/${effect.effect}`} data-effect={`${effect.plugin}/${effect.effect}`} className="flex flex-col gap-3">
            <div className="flex min-h-6 items-center justify-between gap-2">
              <span className="truncate text-[13px] font-medium text-fg">{effect.name}</span>
              <IconButton label={`Remove ${effect.name}`} className={QUIET} onClick={() => set((all) => all.filter((_, i) => i !== index))}>
                <X size={14} />
              </IconButton>
            </div>
            {issue ? (
              <p role="status" className="flex items-start gap-1.5 text-[12px] text-muted">
                <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
                <span>{plainDetail(issue.message)}</span>
              </p>
            ) : (
              def?.params.map((param) => (
                <Control
                  key={param.id}
                  input={param}
                  value={shown(param, effect)}
                  onChange={(v) => set((all) => all.map((e, i) => (i === index ? { ...e, params: { ...e.params, [param.id]: v } } : e)), `${clip.id}:effect:${index}:${param.id}`)}
                />
              ))
            )}
          </div>
        );
      })}
      {menu && (
        <Menu
          items={installed.map((e) => ({ label: named(e), disabled: e.error ? `Has an error: ${plainDetail(e.error)}` : null, run: () => add(e) }))}
          at={{ x: menu.x, y: menu.y, align: "end" }}
          label="Add effect"
          keyboard={menu.keyboard}
          onClose={() => setMenu(null)}
        />
      )}
    </Section>
  );
}

import { useEffect, useState, type MouseEvent } from "react";
import { AlertTriangle, FolderOpen, MoreHorizontal, Puzzle, Sparkles, Store, Trash2 } from "lucide-react";
import { api, errorText } from "../../lib/api";
import { askAi, brokenEffects, changeWithAi, fixable, fixWithAi, loadPlugins, makeWithAi, openPlugin, plainDetail, removePlugin, usePlugins, type Plugin } from "../../lib/plugins";
import { showTab, useGallery } from "../../lib/gallery";
import { useEditor } from "../../lib/store";
import { Button, IconButton, Menu, TabBar, TabPanel, lockedProps, useLockReason, type MenuEntry } from "../ui";
import { GalleryList } from "./GalleryList";

const SUGGESTIONS = ["Make a plugin that removes clips shorter than half a second", "Make a plugin that adds a “Follow for more” text at the end", "Make a plugin that puts a title on every new topic"];

/** A plugin has no picture of its own: its first letter on a quiet tile tells the rows apart without colour. */
export function PluginMark({ plugin, size = 30 }: { plugin: Pick<Plugin, "name" | "error">; size?: number }) {
  if (plugin.error)
    return (
      <span className="flex shrink-0 items-center justify-center rounded-lg bg-raised text-danger" style={{ width: size, height: size }} aria-hidden>
        <AlertTriangle size={15} />
      </span>
    );
  return (
    <span className="flex shrink-0 items-center justify-center rounded-lg bg-raised text-[13px] font-semibold text-fg/80 inset-ring inset-ring-white/[.06]" style={{ width: size, height: size }} aria-hidden>
      {plugin.name.trim().charAt(0).toUpperCase() || "?"}
    </span>
  );
}

export const showFolder = (id: string | null) =>
  api.showPluginFolder(id).catch((e) => useEditor.getState().toast({ kind: "error", text: `Couldn't show the folder: ${errorText(e)}` }));

export function pluginMenu(p: Plugin): MenuEntry[] {
  return [
    { label: "Change with AI", icon: <Sparkles size={14} />, run: () => changeWithAi(p) },
    { label: "Show in folder", icon: <FolderOpen size={14} />, run: () => void showFolder(p.id) },
    "separator",
    { label: "Move to Trash", icon: <Trash2 size={14} />, danger: true, run: () => void removePlugin(p) },
  ];
}

function Row({ plugin: p, open }: { plugin: Plugin; open: boolean }) {
  const runs = usePlugins((s) => [s.auto.import.includes(p.id) && "After import", s.auto.export.includes(p.id) && "Before export"].filter(Boolean).join(" · "));
  const broken = brokenEffects(p);
  const [menu, setMenu] = useState<{ x: number; y: number; keyboard: boolean } | null>(null);
  const showMenu = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    setMenu({ x: r.right, y: r.bottom + 4, keyboard: e.detail === 0 });
  };
  return (
    <li className={`group relative flex min-h-12 items-center gap-2.5 rounded-[10px] pr-1 transition-colors duration-[120ms] ease-out ${open ? "bg-white/[.08]" : "hover:bg-white/[.05]"}`}>
      <button
        type="button"
        data-plugin={p.id}
        aria-current={open || undefined}
        onClick={() => openPlugin(p.id)}
        className="flex min-w-0 flex-1 items-center gap-2.5 self-stretch rounded-[10px] py-2 pl-2 text-left"
      >
        <PluginMark plugin={p} />
        <span className="flex min-w-0 flex-col leading-[17px]">
          <span className="truncate font-medium text-fg">{p.name}</span>
          {p.error ? (
            <span className="line-clamp-2 text-[12px] text-danger">
              {runs && <span className="text-muted">{runs} · </span>}Can’t load: {plainDetail(p.error)}
            </span>
          ) : broken.length > 0 ? (
            <span className="truncate text-[12px] text-warn">
              {runs && <span className="text-muted">{runs} · </span>}Can’t draw {broken.map((e) => e.name).join(", ")}
            </span>
          ) : (
            <span className="truncate text-[12px] text-muted">{[runs, p.description].filter(Boolean).join(" · ")}</span>
          )}
        </span>
      </button>
      {p.error && fixable(p.error) && (
        <Button variant="ghost" className="h-7 shrink-0 gap-1 px-2 text-[12px] text-accent! hover:text-accent!" onClick={() => fixWithAi(p, p.error ?? "")}>
          <Sparkles size={13} /> Fix with AI
        </Button>
      )}
      {!p.error && broken.length > 0 && (
        <Button variant="ghost" className="h-7 shrink-0 gap-1 px-2 text-[12px] text-accent! hover:text-accent!" onClick={() => fixWithAi(p, broken.map((e) => e.error).join("\n"))}>
          <Sparkles size={13} /> Fix with AI
        </Button>
      )}
      <IconButton label={`More for ${p.name}`} className={`size-7 shrink-0 ${open || menu ? "" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100"}`} onClick={showMenu}>
        <MoreHorizontal size={16} />
      </IconButton>
      {menu && <Menu items={pluginMenu(p)} at={{ x: menu.x, y: menu.y, align: "end" }} label={p.name} keyboard={menu.keyboard} onClose={() => setMenu(null)} />}
    </li>
  );
}

function Empty() {
  const lock = useLockReason();
  return (
    <div className="flex flex-col items-center px-5 pt-8 pb-4 text-center">
      <Puzzle size={30} strokeWidth={1.6} className="text-muted" aria-hidden />
      <h3 className="mt-2.5 text-[13px] font-semibold text-fg">No plugins yet</h3>
      <p className="mt-1 mb-3.5 max-w-[260px] text-[12px] text-muted">Tell the AI what you’re missing. It writes a plugin that shows up here.</p>
      <Button variant="primary" pill className="h-8 gap-1.5 px-4" onClick={makeWithAi}>
        <Sparkles size={14} /> Make a plugin
      </Button>
      <p className="mt-6 mb-1.5 self-stretch text-left text-[12px] text-muted">Try</p>
      <ul className="flex flex-col gap-1.5 self-stretch">
        {SUGGESTIONS.map((s) => (
          <li key={s}>
            <button
              type="button"
              {...lockedProps(lock)}
              onClick={lock ? undefined : () => askAi(s)}
              className="flex w-full items-center gap-2 rounded-[9px] bg-white/[.05] px-2.5 py-2 text-left text-[12px] text-fg transition-colors duration-[120ms] ease-out hover:bg-white/[.09] aria-disabled:cursor-not-allowed aria-disabled:opacity-40"
            >
              <Sparkles size={12} className="shrink-0 text-accent" /> {s.replace("Make a plugin that ", "")}
            </button>
          </li>
        ))}
      </ul>
      <div className="mt-3 flex flex-wrap justify-center gap-1">
        <Button variant="ghost" className="h-7 gap-1.5 px-2 text-[12px] text-accent! hover:text-accent!" onClick={() => showTab("gallery")}>
          <Store size={13} /> Browse the gallery
        </Button>
        <Button variant="ghost" className="h-7 gap-1.5 px-2 text-[12px] text-accent! hover:text-accent!" onClick={() => void showFolder(null)}>
          <FolderOpen size={13} /> Show plugins folder
        </Button>
      </div>
    </div>
  );
}

/** The plugins in Nuzky's plugins folder; the one chosen opens in the inspector. */
function Installed() {
  const list = usePlugins((s) => s.list);
  const open = usePlugins((s) => s.open);
  useEffect(() => {
    void loadPlugins();
  }, []);
  if (!list) return null;
  if (list.length === 0) return <Empty />;
  return (
    <>
      <div className="flex h-10 shrink-0 items-center justify-between px-3.5">
        <h3 className="text-[13px] font-semibold text-fg">Your plugins</h3>
        <Button variant="ghost" className="h-7 gap-1 px-2 text-[12px] text-accent! hover:text-accent!" onClick={makeWithAi}>
          <Sparkles size={13} /> Make a plugin
        </Button>
      </div>
      <ul className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-1.5 pb-2" aria-label="Plugins">
        {list.map((p) => (
          <Row key={p.id} plugin={p} open={p.id === open} />
        ))}
      </ul>
    </>
  );
}

const TABS = [
  { id: "installed" as const, label: "Installed" },
  { id: "gallery" as const, label: "Gallery" },
];

/** The user's plugins, and the gallery of plugins other people wrote. */
export function PluginsTab() {
  const tab = useGallery((s) => s.tab);
  return (
    <div className="flex min-h-0 flex-1 flex-col pt-3">
      <TabBar group="plugins" label="Plugins" tabs={TABS} value={tab} onChange={showTab} />
      <TabPanel group="plugins" id={tab} className="flex min-h-0 flex-1 flex-col">
        {tab === "installed" ? <Installed /> : <GalleryList />}
      </TabPanel>
    </div>
  );
}

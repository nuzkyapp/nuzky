import { useEffect, useState } from "react";
import { AlertCircle, Check, Loader2 } from "lucide-react";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api, errorText, plainError } from "../../lib/api";
import { useSettings } from "../../lib/settings";
import { useEditor } from "../../lib/store";
import type { StorageGroup, StorageGroupId, StorageUsage } from "../../lib/types";
import { Button } from "../ui";
import { formatBytes } from "./parts";

const GROUPS: Record<StorageGroupId, { title: string; detail: string; colour: string }> = {
  sound: { title: "Prepared sound of your videos", detail: "Made again when a video plays or exports", colour: "bg-white/80" },
  pictures: { title: "Preview copies, cut-outs and project pictures", detail: "Made again as you work", colour: "bg-white/55" },
  youtube: { title: "Videos from YouTube", detail: "What Nuzky downloaded to cut a part from", colour: "bg-white/35" },
  models: { title: "Models", detail: "Remove one in Models", colour: "bg-white/20" },
  data: { title: "Projects, sounds, plugins and your style", detail: "Never cleared here", colour: "bg-white/10" },
};

const CLEARABLE = ["sound", "pictures", "youtube"] as const;
type Clearable = (typeof CLEARABLE)[number];

/** Settings › Storage: how much of the disk Nuzky takes, and Clear for what it makes again by itself. */
export function StoragePane() {
  const [usage, setUsage] = useState<StorageUsage | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [clearing, setClearing] = useState<Clearable | null>(null);
  const [cleared, setCleared] = useState<Partial<Record<Clearable, number>>>({});
  // A job that starts or ends changes what may be cleared.
  const jobs = useEditor((s) =>
    Object.values(s.jobs)
      .map((j) => `${j.id}:${j.status}`)
      .join(),
  );

  useEffect(() => {
    let live = true;
    api.storageUsage().then(
      (u) => live && setUsage(u),
      (e) => live && setError(plainError(errorText(e))),
    );
    return () => {
      live = false;
    };
  }, [jobs]);

  const clear = async (id: Clearable, before: number) => {
    setClearing(id);
    setError(null);
    try {
      const after = await api.clearStorage(id);
      setUsage(after);
      const left = after.groups.find((g) => g.id === id)?.bytes ?? 0;
      setCleared((c) => ({ ...c, [id]: Math.max(0, before - left) }));
    } catch (e) {
      setError(plainError(errorText(e)));
    } finally {
      setClearing(null);
    }
  };

  if (!usage)
    return error ? (
      <p className="flex items-start gap-1.5 text-[12px] text-fg" role="alert">
        <AlertCircle size={14} className="mt-px shrink-0 text-danger" />
        Couldn't measure the storage: {error}
      </p>
    ) : (
      <p role="status" className="flex items-center gap-2 text-[12px] text-muted">
        <Loader2 size={14} className="animate-spin" /> Measuring…
      </p>
    );

  const total = usage.groups.reduce((sum, g) => sum + g.bytes, 0);
  const shown = usage.groups.filter((g) => g.id in GROUPS);
  return (
    <div data-setting="storage" className="flex flex-col gap-4">
      <div className="flex flex-col gap-2">
        <p className="tabular text-[13px] text-fg">{formatBytes(total)} on this computer</p>
        <div className="flex h-2 gap-px overflow-hidden rounded-full bg-white/[.04]" role="img" aria-label={shown.map((g) => `${GROUPS[g.id].title}: ${formatBytes(g.bytes)}`).join(", ")}>
          {total > 0 && shown.map((g) => <span key={g.id} className={`h-full ${GROUPS[g.id].colour}`} style={{ width: `${(g.bytes / total) * 100}%` }} />)}
        </div>
      </div>
      <div className="flex flex-col rounded-xl bg-white/[.055] shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)]">
        {shown.map((g) => (
          <Row key={g.id} group={g} clearing={clearing} cleared={cleared[g.id as Clearable]} onClear={clear} />
        ))}
      </div>
      {error && (
        <p className="flex items-start gap-1.5 text-[12px] text-fg" role="alert">
          <AlertCircle size={14} className="mt-px shrink-0 text-danger" />
          {error}
        </p>
      )}
      <p className="text-[12px] text-muted">Clear keeps what the open project uses and never touches your videos. Nuzky makes the rest again when it needs it.</p>
    </div>
  );
}

function Row({ group: g, clearing, cleared, onClear }: { group: StorageGroup; clearing: Clearable | null; cleared?: number; onClear: (id: Clearable, before: number) => void }) {
  const { title, detail, colour } = GROUPS[g.id];
  const id = (CLEARABLE as readonly string[]).includes(g.id) ? (g.id as Clearable) : null;
  const kept = g.keptBytes > 0 ? ` · ${formatBytes(g.keptBytes)} kept for the open project` : "";
  return (
    <div data-storage={g.id} className="flex min-h-[44px] items-center gap-3 border-t border-white/[.06] px-3.5 py-2.5 first:border-t-0">
      <span aria-hidden className={`size-2 shrink-0 rounded-full ${colour} shadow-[inset_0_0_0_1px_rgb(255_255_255/.2)]`} />
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <span className="text-[13px] font-medium text-fg">{title}</span>
        <span className="tabular text-[12px] text-muted">
          {cleared !== undefined ? (
            <span className="text-fg">
              <Check size={12} strokeWidth={3} className="mr-1 inline align-[-1px] text-ok" />
              Cleared {formatBytes(cleared)}
              {kept}
            </span>
          ) : (
            `${formatBytes(g.bytes)} · ${detail}${kept}`
          )}
        </span>
        {id && g.blocked && <span className="text-[12px] text-fg">{g.blocked}</span>}
      </div>
      {id ? (
        <Button
          pill
          disabled={!!g.blocked || clearing !== null || g.bytes - g.keptBytes <= 0}
          disabledReason={g.blocked ?? (clearing ? "Clearing…" : "Nothing to clear")}
          onClick={() => onClear(id, g.bytes)}
        >
          {clearing === id ? "Clearing…" : "Clear"}
        </Button>
      ) : g.id === "models" ? (
        <Button pill onClick={() => useSettings.setState({ pane: "models", target: "speech" })}>
          Models
        </Button>
      ) : (
        g.path && (
          <Button pill onClick={() => void revealItemInDir(g.path!)}>
            Show in folder
          </Button>
        )
      )}
    </div>
  );
}

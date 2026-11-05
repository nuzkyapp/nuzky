import { useEffect, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { api, errorText, plainError } from "../../lib/api";
import { loadPlugins, revokePlugin, setAuto, usePlugins, type Plugin, type Trigger } from "../../lib/plugins";
import { Button, Checkbox } from "../ui";
import { Group, Item } from "./parts";

const TRIGGERS: [Trigger, string][] = [
  ["import", "After import"],
  ["export", "Before export"],
];

/** Settings › Plugins: which plugins run by themselves and when, and which may reach the user's files, the internet and other apps. */
export function PluginsPane() {
  const list = usePlugins((s) => s.list);
  const auto = usePlugins((s) => s.auto);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => void loadPlugins(), []);
  if (!list) return null;

  const runnable = list.filter((p) => p.runs);
  const full = list.filter((p) => p.fullAccess);
  const revoke = async (p: Plugin) => {
    setError(null);
    try {
      await revokePlugin(p);
    } catch (e) {
      setError(plainError(errorText(e)));
    }
  };

  return (
    <div className="flex flex-col gap-6">
      <Group title="Run automatically" id="plugins-auto" note={runnable.length ? "Each runs on the whole project with the settings it last had." : undefined}>
        {runnable.length === 0 ? (
          <Item title="No plugins that run" detail="Add one in the Plugins tab of the editor's library, or ask your AI to make one." />
        ) : (
          <table className="w-full text-left">
            <thead>
              <tr className="text-[12px] text-muted">
                <th className="px-3.5 pt-2.5 pb-1.5 font-normal">Plugin</th>
                {TRIGGERS.map(([, label]) => (
                  <th key={label} className="w-[104px] pt-2.5 pb-1.5 text-center font-normal">
                    {label}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {runnable.map((p) => (
                <tr key={p.id} data-plugin={p.id} className="border-t border-white/[.06]">
                  <td className="px-3.5 py-2.5 text-[13px] font-medium text-fg">{p.name}</td>
                  {TRIGGERS.map(([trigger, label]) => (
                    <td key={trigger} className="text-center">
                      <span className="inline-flex">
                        <Checkbox label={`${p.name}: run ${label.toLowerCase()}`} hideLabel checked={auto[trigger].includes(p.id)} onChange={(on) => setAuto(p.id, trigger, on)} />
                      </span>
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Group>
      <Group
        title="Full access"
        id="plugins-access"
        note="A plugin with full access can read, change and delete your files, use the internet and run other apps. Nuzky asks before its first run and again after it changes."
      >
        {full.length === 0 ? (
          <Item title="No plugin asks for full access" />
        ) : (
          full.map((p) => (
            <Item
              key={p.id}
              title={
                <span className="flex items-center gap-1.5">
                  <AlertTriangle size={13} className="shrink-0 text-warn" />
                  {p.name}
                </span>
              }
              detail={p.allowed ? "Allowed. It runs without asking until it changes." : "Asks you before its next run"}
            >
              {p.allowed && (
                <Button pill onClick={() => void revoke(p)}>
                  Revoke
                </Button>
              )}
            </Item>
          ))
        )}
      </Group>
      {error && (
        <p className="-mt-3 flex items-start gap-1.5 text-[12px] text-fg" role="alert">
          <AlertTriangle size={14} className="mt-px shrink-0 text-danger" />
          {error}
        </p>
      )}
      <div>
        <Button pill onClick={() => void api.showPluginFolder(null)}>
          Show plugins folder
        </Button>
      </div>
    </div>
  );
}

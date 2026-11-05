import { Check, Loader2 } from "lucide-react";
import { closeSettings } from "../../lib/settings";
import { SETUP_STEPS, useSetup } from "../../lib/setup";
import { trySample } from "../../lib/tour";
import { checkForUpdates, downloadUpdate, setAutoCheck, useUpdates } from "../../lib/updates";
import { Button, Switch } from "../ui";
import { UpdateNuzkyButton } from "../Updates";
import { Group, Item } from "./parts";

/** Settings › General: which Nuzky this is and its updates, and the ways to get started again. */
export function GeneralPane() {
  const { version, enabled, auto, latest, checking, install } = useUpdates();
  const done = useSetup((s) => SETUP_STEPS.filter((step) => s.states[step] === "done").length);
  const busy = checking ? "Checking for updates…" : install ? "The update is on its way." : undefined;

  return (
    <div className="flex flex-col gap-6">
      <Group title="Updates" id="updates">
        <Item
          title={`Nuzky ${version}`}
          detail={
            !enabled ? (
              "This copy doesn't update itself."
            ) : checking === "manual" ? (
              <span className="flex items-center gap-1">
                <Loader2 size={12} className="shrink-0 animate-spin" /> Checking for updates…
              </span>
            ) : latest ? (
              `Nuzky ${latest} is available.`
            ) : (
              <span className="flex items-center gap-1">
                <Check size={12} strokeWidth={3} className="shrink-0 text-ok" /> Up to date
              </span>
            )
          }
        >
          {enabled && latest ? (
            <UpdateNuzkyButton />
          ) : (
            enabled && (
              <Button pill disabled={!!busy} disabledReason={busy} onClick={() => void checkForUpdates(true)}>
                Check now
              </Button>
            )
          )}
        </Item>
        {enabled && (
          <Item id="auto-update" title="Check automatically" detail="Once a day. Nuzky reads only the newest version number.">
            <Switch label="Check for updates automatically" checked={auto} onChange={setAutoCheck} />
          </Item>
        )}
        {enabled && latest && (
          <div className="border-t border-white/[.06] px-3.5 py-2">
            <button type="button" onClick={downloadUpdate} className="text-[12px] font-medium text-accent hover:underline">
              What's new in Nuzky {latest}…
            </button>
          </div>
        )}
      </Group>
      <Group title="Getting started">
        <Item id="setup" title="Get set up" detail={`${done} of ${SETUP_STEPS.length} done`}>
          <Button
            pill
            onClick={() => {
              closeSettings();
              useSetup.getState().openSetup();
            }}
          >
            Open
          </Button>
        </Item>
        <Item id="tour" title="Take the tour" detail="On a new copy of the sample video">
          <Button
            pill
            onClick={() => {
              closeSettings();
              trySample();
            }}
          >
            Start
          </Button>
        </Item>
      </Group>
    </div>
  );
}

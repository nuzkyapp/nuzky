import { useState, type MouseEvent } from "react";
import { CircleArrowDown, Loader2, RotateCw } from "lucide-react";
import { useWindowSize } from "../lib/dock";
import { cancelUpdate, checkForUpdates, downloadUpdate, installUpdate, restartBlocker, restartToUpdate, setAutoCheck, useRestartBlocker, useUpdates, waitFor } from "../lib/updates";
import { Button, IconButton, Menu, type MenuEntry } from "./ui";

type ShowMenu = (items: MenuEntry[], at: { x: number; y: number; above?: boolean }, label: string, keyboard: boolean) => void;

/** Updating when there is something newer (or its download where this copy can't), stopping its download or restarting into it; what's new; a check now; the automatic check on or off. */
function updateMenu(): MenuEntry[] {
  const { latest, canInstall, checking, auto, install } = useUpdates.getState();
  const blocked = restartBlocker();
  const update: MenuEntry[] = !latest
    ? []
    : !canInstall
      ? [{ label: `Download Nuzky ${latest}…`, icon: <CircleArrowDown size={15} />, run: downloadUpdate }, "separator"]
      : [
          install?.stage === "downloading"
            ? { label: "Stop downloading", run: cancelUpdate }
            : install
              ? { label: blocked ? waitFor(blocked) : "Restart to update", icon: <RotateCw size={15} />, disabled: blocked && `${waitFor(blocked)}.`, run: () => void restartToUpdate() }
              : { label: `Update to Nuzky ${latest}`, icon: <CircleArrowDown size={15} />, run: () => void installUpdate() },
          { label: `What's new in Nuzky ${latest}…`, run: downloadUpdate },
          "separator",
        ];
  const busy = checking ? "Checking for updates…" : install?.stage === "downloading" ? "The update is downloading." : install ? `Nuzky ${latest} is ready to install.` : null;
  return [...update, { label: "Check for updates", disabled: busy, run: () => void checkForUpdates(true) }, { label: "Check automatically", checked: auto, run: () => setAutoCheck(!auto) }];
}

/**
 * The foot of the home sidebar: this version, or the newer one to update to in an accent row, which
 * shows the download's progress beside its Stop, then Restart to update, or why it waits. Its menu
 * updates, stops the download, restarts, checks for updates and turns the automatic check on or off.
 * Nuzky starts on the home screen, so a new version is seen at the next launch.
 */
export function VersionRow({ showMenu }: { showMenu: ShowMenu }) {
  const { version, enabled, latest, checking, install } = useUpdates();
  const blocked = useRestartBlocker();
  if (!version) return null;
  if (!enabled) return <p className="flex h-8 items-center px-2.5 text-[12px] text-muted">Nuzky {version}</p>;

  const accent = "bg-accent/[.16] shadow-[inset_0_0_0_1px_rgb(41_151_255/.45)]";
  // Installing takes a moment and offers nothing to choose.
  if (install?.stage === "installing")
    return (
      <p data-version role="status" className={`flex h-9 items-center gap-2.5 rounded-lg px-2.5 text-[13px] font-semibold text-fg ${accent}`}>
        <Loader2 size={16} className="shrink-0 animate-spin text-accent" /> Installing update…
      </p>
    );

  const open = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(updateMenu(), { x: r.left, y: r.top - 6, above: true }, "Updates", e.detail === 0);
  };
  const found = !!latest && (checking !== "manual" || !!install);
  const downloading = install?.stage === "downloading" ? install : null;

  return (
    <div className="relative">
      <button
        type="button"
        data-version
        aria-haspopup="menu"
        title={install ? `Nuzky ${latest}` : latest ? `Nuzky ${latest} is available. You have ${version}.` : "Updates"}
        onClick={open}
        className={`flex w-full shrink-0 items-center gap-2.5 rounded-lg pl-2.5 text-left text-[13px] transition-colors duration-[120ms] ease-out ${
          found ? `min-h-9 py-1.5 hover:bg-accent/[.22] ${accent}` : "h-8 hover:bg-white/[.05]"
        } ${downloading ? "pr-14" : "pr-2.5"}`}
      >
        {downloading ? (
          <>
            <Loader2 size={16} className="shrink-0 animate-spin text-accent" />
            <span className="min-w-0 flex-1 truncate font-semibold text-fg" role="status">
              Downloading
              {downloading.percent !== null && <span className="tabular ml-1.5 text-[12px] font-medium text-accent">{downloading.percent}%</span>}
            </span>
          </>
        ) : install ? (
          <>
            <RotateCw size={16} className={`shrink-0 ${blocked ? "text-muted" : "text-accent"}`} />
            {blocked ? <span className="min-w-0 flex-1 font-semibold text-muted">{waitFor(blocked)}</span> : <VersionLabel label="Restart to update" version={latest} />}
          </>
        ) : checking === "manual" ? (
          <>
            <Loader2 size={16} className="shrink-0 animate-spin text-muted" />
            <span className="min-w-0 flex-1 truncate text-muted">Checking for updates…</span>
          </>
        ) : found ? (
          <>
            <CircleArrowDown size={16} className="shrink-0 text-accent" />
            <VersionLabel label="Update available" version={latest} />
          </>
        ) : (
          <span className="min-w-0 flex-1 truncate text-[12px] text-muted">Nuzky {version}</span>
        )}
      </button>
      {downloading && (
        <Button pill title="Stop downloading the update" onClick={cancelUpdate} className="absolute top-1/2 right-1.5 h-6 -translate-y-1/2 px-2.5 text-[12px]">
          Stop
        </Button>
      )}
    </div>
  );
}

/** The version goes under the label: beside it, one like 0.1.1-alpha leaves the label no room in the sidebar. */
function VersionLabel({ label, version }: { label: string; version: string | null }) {
  return (
    <span className="flex min-w-0 flex-1 flex-col">
      <span className="truncate font-semibold leading-[17px] text-fg">{label}</span>
      <span className="tabular truncate text-[12px] font-medium leading-4 text-accent">{version}</span>
    </span>
  );
}

/**
 * In the editor's top bar, only while a newer version is out: a quiet button with the same menu;
 * while it downloads, the progress with Stop, as other downloads show it; then Restart to update,
 * or why it waits. Below 1280 px the project's name needs the room, so they lose their words.
 */
export function UpdateButton() {
  const latest = useUpdates((s) => (s.enabled ? s.latest : null));
  const install = useUpdates((s) => s.install);
  const blocked = useRestartBlocker();
  const compact = useWindowSize().w < 1280;
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; keyboard: boolean; back: HTMLElement } | null>(null);
  if (!latest) return null;
  if (install?.stage === "downloading" || install?.stage === "installing")
    return (
      <span data-update-progress className={`bar flex h-8 shrink-0 items-center gap-1.5 rounded-full pr-1 text-[12px] font-medium text-fg ${compact ? "pl-2.5" : "pl-3"}`} role="status">
        <Loader2 size={14} className="shrink-0 animate-spin text-accent" />
        {install.stage === "downloading" ? (
          <>
            <span className={compact ? "sr-only" : ""}>Downloading update</span>
            {/* A fixed width, so the capsule keeps its size while the percentage grows. */}
            <span className="tabular w-[34px] shrink-0 text-right text-muted">{install.percent !== null && `${install.percent}%`}</span>
            <Button pill className="h-6 shrink-0 px-2.5 text-[12px]" title="Stop downloading the update" onClick={cancelUpdate}>
              Stop
            </Button>
          </>
        ) : (
          <span className="pr-2">Installing update…</span>
        )}
      </span>
    );
  if (install && blocked)
    return (
      <span
        data-update-waiting
        role="status"
        title={`Nuzky ${latest} is ready. ${waitFor(blocked)}.`}
        className={`bar flex h-8 shrink-0 items-center justify-center gap-1.5 rounded-full text-[12px] font-medium text-muted ${compact ? "w-8" : "px-3.5"}`}
      >
        <RotateCw size={14} className="shrink-0" />
        <span className={compact ? "sr-only" : ""}>{waitFor(blocked)}</span>
      </span>
    );
  if (install)
    return compact ? (
      <div className="bar flex rounded-full">
        <IconButton round data-update-button label={`Restart to update to Nuzky ${latest}`} onClick={() => void restartToUpdate()}>
          <RotateCw size={16} className="text-accent" />
        </IconButton>
      </div>
    ) : (
      <Button variant="bar" pill data-update-button title={`Restart to update to Nuzky ${latest}`} onClick={() => void restartToUpdate()} className="text-[12px]">
        <RotateCw size={14} className="text-accent" /> Restart to update
      </Button>
    );
  return (
    <div className="bar flex rounded-full">
      <IconButton
        round
        data-update-button
        aria-haspopup="menu"
        aria-expanded={!!menu}
        label={`Nuzky ${latest} is available`}
        onClick={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          setMenu({ at: { x: r.left, y: r.bottom + 6 }, keyboard: e.detail === 0, back: e.currentTarget });
        }}
      >
        <CircleArrowDown size={16} className="text-accent" />
      </IconButton>
      {menu && (
        <Menu
          items={updateMenu()}
          at={menu.at}
          label="Updates"
          keyboard={menu.keyboard}
          onClose={(chose) => {
            const back = menu.back;
            setMenu(null);
            if (!chose) back.focus();
            // After Update the button gives way to the progress, whose Stop takes the focus; otherwise it goes back to the button, not to the page.
            else
              requestAnimationFrame(() => {
                if (document.activeElement !== document.body) return;
                (back.isConnected ? back : document.querySelector<HTMLElement>("[data-update-progress] button"))?.focus();
              });
          }}
        />
      )}
    </div>
  );
}

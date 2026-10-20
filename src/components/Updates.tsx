import { useState, type MouseEvent } from "react";
import { CircleArrowDown, Loader2 } from "lucide-react";
import { cancelUpdate, checkForUpdates, downloadUpdate, installBlocked, installUpdate, setAutoCheck, useUpdates } from "../lib/updates";
import { Button, IconButton, Menu, type MenuEntry } from "./ui";

type ShowMenu = (items: MenuEntry[], at: { x: number; y: number; above?: boolean }, label: string, keyboard: boolean) => void;

/** Installing when there is something newer, or stopping its download; what's new; a check now; the automatic check on or off. */
function updateMenu(): MenuEntry[] {
  const { latest, checking, auto, install } = useUpdates.getState();
  const update: MenuEntry[] = !latest
    ? []
    : [
        install && install.percent !== 100
          ? { label: "Stop downloading", run: cancelUpdate }
          : {
              label: `Install Nuzky ${latest} and restart`,
              icon: <CircleArrowDown size={15} />,
              disabled: install ? "Installing the update…" : installBlocked(),
              run: () => void installUpdate(),
            },
        { label: `What's new in Nuzky ${latest}…`, run: downloadUpdate },
        "separator",
      ];
  return [
    ...update,
    { label: "Check for updates", disabled: checking ? "Checking for updates…" : null, run: () => void checkForUpdates(true) },
    { label: "Check automatically", checked: auto, run: () => setAutoCheck(!auto) },
  ];
}

/** Shown with the percentage while it downloads; then it checks and installs. The sidebar row has room for less. */
function installText(percent: number | null, short = false) {
  return percent === 100 ? "Installing update…" : short ? "Downloading" : "Downloading update";
}

/**
 * The foot of the home sidebar: this version, or the newer one to install in an accent row, which
 * shows the download's progress while it installs. Its menu installs, stops the download, checks for
 * updates and turns the automatic check on or off. Nuzky starts on the home screen, so a new version is
 * seen at the next launch.
 */
export function VersionRow({ showMenu }: { showMenu: ShowMenu }) {
  const { version, enabled, latest, checking, install } = useUpdates();
  if (!version) return null;
  if (!enabled) return <p className="flex h-8 items-center px-2.5 text-[12px] text-muted">Nuzky {version}</p>;

  const open = (e: MouseEvent<HTMLButtonElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    showMenu(updateMenu(), { x: r.left, y: r.top - 6, above: true }, "Updates", e.detail === 0);
  };
  const found = !!latest && (checking !== "manual" || !!install);

  return (
    <button
      type="button"
      data-version
      aria-haspopup="menu"
      title={install ? installText(install.percent) : latest ? `Nuzky ${latest} is available. You have ${version}.` : "Updates"}
      onClick={open}
      className={`flex w-full shrink-0 items-center gap-2.5 rounded-lg px-2.5 text-left text-[13px] transition-colors duration-[120ms] ease-out ${
        found ? "h-9 bg-accent/[.16] shadow-[inset_0_0_0_1px_rgb(41_151_255/.45)] hover:bg-accent/[.22]" : "h-8 hover:bg-white/[.05]"
      }`}
    >
      {install ? (
        <>
          <Loader2 size={16} className="shrink-0 animate-spin text-accent" />
          <span className="min-w-0 flex-1 truncate font-semibold text-fg" role="status">
            {installText(install.percent, true)}
          </span>
          {install.percent !== null && install.percent < 100 && <span className="tabular shrink-0 text-[12px] font-medium text-accent">{install.percent}%</span>}
        </>
      ) : checking === "manual" ? (
        <>
          <Loader2 size={16} className="shrink-0 animate-spin text-muted" />
          <span className="min-w-0 flex-1 truncate text-muted">Checking for updates…</span>
        </>
      ) : found ? (
        <>
          <CircleArrowDown size={16} className="shrink-0 text-accent" />
          <span className="min-w-0 flex-1 truncate font-semibold text-fg">Update available</span>
          <span className="tabular shrink-0 text-[12px] font-medium text-accent">{latest}</span>
        </>
      ) : (
        <span className="min-w-0 flex-1 truncate text-[12px] text-muted">Nuzky {version}</span>
      )}
    </button>
  );
}

/**
 * In the editor's top bar, only while a newer version is out: a quiet button with the same menu, or
 * while it installs, the progress with Stop, as other downloads show it.
 */
export function UpdateButton() {
  const latest = useUpdates((s) => (s.enabled ? s.latest : null));
  const install = useUpdates((s) => s.install);
  const [menu, setMenu] = useState<{ at: { x: number; y: number }; keyboard: boolean; back: HTMLElement } | null>(null);
  if (!latest) return null;
  if (install)
    return (
      <span data-update-progress className="bar flex h-8 items-center gap-1.5 rounded-full pr-1 pl-3 text-[12px] font-medium text-fg" role="status">
        <Loader2 size={14} className="shrink-0 animate-spin text-accent" />
        <span>{installText(install.percent)}</span>
        {install.percent !== null && install.percent < 100 && <span className="tabular text-muted">{install.percent}%</span>}
        {install.percent !== 100 && (
          <Button pill className="h-6 shrink-0 px-2.5 text-[12px]" title="Stop downloading the update" onClick={cancelUpdate}>
            Stop
          </Button>
        )}
      </span>
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
            // After Install the button gives way to the progress, whose Stop takes the focus; otherwise it goes back to the button, not to the page.
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

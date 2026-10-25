import { useEffect, useRef, useState, type KeyboardEvent } from "react";
import { AlertCircle, WifiOff, X } from "lucide-react";
import { galleryError, installGalleryPlugin, openGalleryPlugin, REFUSED, size, stopInstall, useGallery, type GalleryPlugin } from "../../lib/gallery";
import { openPlugin, usePlugins } from "../../lib/plugins";
import { PluginMark } from "../panel/PluginsTab";
import { UpdateNuzkyButton } from "../Updates";
import { Button, IconButton, ProgressBar, Section } from "../ui";

/** The plugin whose button the user just used, so the button that takes its place also takes the focus. */
let refocus: string | null = null;

/** Installing, updating or replacing, with what it would replace said first. Installing changes no project, so the AI lock leaves it alone. */
function Action({ plugin: p }: { plugin: GalleryPlugin }) {
  const installing = useGallery((s) => s.installing);
  const failed = useGallery((s) => (s.failed?.id === p.id ? s.failed.text : null));
  const own = usePlugins((s) => s.list?.find((q) => q.id === p.id)?.name) ?? p.name;
  const [confirming, setConfirming] = useState(false);
  const box = useRef<HTMLDivElement>(null);
  const choice = useRef<HTMLDivElement>(null);
  // The safe answer has the focus, so Enter keeps the user's version.
  useEffect(() => {
    if (confirming) choice.current?.querySelector("button")?.focus();
  }, [confirming]);
  // Install, Stop and Cancel take their own button away, and a new state remounts this; the keyboard goes on from the button in its place.
  useEffect(() => {
    if (refocus !== p.id || confirming) return;
    const active = document.activeElement;
    if (active && active !== document.body && !box.current?.contains(active)) {
      refocus = null;
      return;
    }
    const main = box.current?.querySelector<HTMLElement>("[data-gallery-main]");
    if (main && main !== active) main.focus();
    if (installing?.id !== p.id) refocus = null;
  });
  const act = (run: () => void) => () => {
    refocus = p.id;
    run();
  };
  const cancel = act(() => setConfirming(false));
  const install = act(() => {
    setConfirming(false);
    void installGalleryPlugin(p, p.state === "available" ? null : p.state);
  });

  let body;
  if (installing?.id === p.id)
    body = (
      <div className="flex flex-col gap-2" data-gallery-progress>
        <div className="flex items-center justify-between gap-2 text-[12px] text-muted">
          <span className="tabular">Installing… {installing.percent}%</span>
          <Button data-gallery-main className="h-7 text-[12px]" onClick={act(stopInstall)}>
            Stop
          </Button>
        </div>
        <ProgressBar value={installing.percent / 100} label={`Installing ${p.name}`} />
      </div>
    );
  else if (p.state === "refused") body = <p className="text-[12px] text-muted">{p.files.some((f) => f.refused) ? REFUSED : "Nuzky won’t install this plugin. Something in it isn’t allowed in plugins."}</p>;
  else if (p.state === "newer")
    body = (
      <div className="flex flex-col gap-2.5">
        <p className="text-[12px] text-muted">This plugin needs a newer Nuzky.</p>
        <UpdateNuzkyButton className="h-8 w-full" />
      </div>
    );
  else {
    const clash = p.state === "clash";
    const replace = clash || p.state === "changed";
    const note =
      p.state === "update"
        ? `You have ${p.installedVersion ? `version ${p.installedVersion}` : "an older version"}. Updating replaces it.`
        : p.state === "changed"
          ? "You or your AI edited your copy."
          : clash
            ? `Your own plugin “${own}” has the same name.`
            : p.state === "installed"
              ? "This version is installed."
              : null;
    const label = clash ? "Replace your plugin" : p.state === "changed" ? "Replace your edited copy" : p.state === "update" ? "Update" : "Install";
    const problem = failed ? galleryError(failed, true) : null;
    const busy = installing ? "Another plugin is installing. Wait for it to finish." : undefined;
    const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      cancel();
    };
    body = (
      <div className="flex flex-col gap-2.5">
        {note && <p className="text-[12px] text-muted">{note}</p>}
        {confirming ? (
          <>
            <p className="text-[12px] text-fg" role="alert">
              {clash ? `Your plugin “${own}” goes to the Trash. You can put it back from there.` : "Your edited copy goes to the Trash, with your changes."}
            </p>
            <div ref={choice} className="flex gap-2" onKeyDown={onKeyDown}>
              <Button pill className="h-8 flex-1" onClick={cancel}>
                Cancel
              </Button>
              <Button variant="primary" pill className="h-8 flex-1 px-3" onClick={install}>
                {label}
              </Button>
            </div>
          </>
        ) : p.state === "installed" ? (
          <Button data-gallery-main pill className="h-8 w-full" onClick={() => openPlugin(p.id)}>
            Open
          </Button>
        ) : (
          <Button data-gallery-main variant={replace ? "secondary" : "primary"} pill className="h-8 w-full" disabled={!!busy} disabledReason={busy} onClick={replace ? () => setConfirming(true) : install}>
            {problem?.retry ? "Try again" : label}
          </Button>
        )}
        {problem && failed && !confirming && (
          <div className="flex items-start gap-2 text-[12px] text-muted" role="alert">
            {failed.startsWith("OFFLINE") ? <WifiOff size={14} className="mt-px shrink-0" /> : <AlertCircle size={14} className="mt-px shrink-0 text-warn" />}
            <span className="min-w-0 flex-1 select-text">
              <span className="text-fg">{problem.title}.</span> {problem.detail}
            </span>
          </div>
        )}
      </div>
    );
  }
  return <div ref={box}>{body}</div>;
}

/** A plugin of the gallery: what Nuzky lets it do, the button, then what its author says and its files. */
export function GalleryInspector({ plugin: p }: { plugin: GalleryPlugin }) {
  return (
    <>
      <div className="flex shrink-0 items-start gap-2.5 px-4 pt-4 pb-3">
        <PluginMark plugin={{ name: p.name, error: null }} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[15px] leading-5 font-semibold tracking-[-0.015em] text-fg">{p.name}</div>
          <div className="mt-[3px] truncate text-[12px] text-muted">
            {p.author ? `By ${p.author}` : "From the gallery"}
            {p.version && ` · ${p.version}`}
          </div>
        </div>
        <IconButton label="Close plugin" className="size-7 shrink-0" onClick={() => openGalleryPlugin(null)}>
          <X size={16} />
        </IconButton>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto border-t border-white/[.07]" data-gallery-detail={p.id}>
        {p.summary.length > 0 && (
          <Section title="What Nuzky lets it do">
            <ul className="flex flex-col gap-1.5 text-[12px] text-fg/85" data-gallery-summary>
              {p.summary.map((line) => (
                <li key={line.text} data-new={line.new || undefined}>
                  {line.new && <span className="mr-1.5 font-medium text-accent">New</span>}
                  {line.text}
                </li>
              ))}
            </ul>
          </Section>
        )}
        <section className="border-b border-white/[.07] px-4 py-4">
          <Action key={p.state} plugin={p} />
        </section>
        {p.description && (
          <Section title="From the author">
            <p className="text-[12px] whitespace-pre-line text-fg/85 select-text" data-gallery-author>
              {p.description}
            </p>
          </Section>
        )}
        <Section title="Files">
          <ul className="flex flex-col gap-1 text-[12px]" data-gallery-files>
            {p.files.map((f) => (
              <li key={f.path} className={`flex justify-between gap-3 ${f.refused ? "text-warn" : ""}`} data-refused={f.refused || undefined}>
                <span className={`min-w-0 truncate ${f.refused ? "" : "text-fg/85"}`}>{f.path}</span>
                <span className={`tabular shrink-0 ${f.refused ? "" : "text-muted"}`}>{f.refused ? "Not allowed" : size(f.size)}</span>
              </li>
            ))}
          </ul>
        </Section>
      </div>
    </>
  );
}

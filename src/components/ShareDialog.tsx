import { useEffect, useRef } from "react";
import { AlertCircle, QrCode, X } from "lucide-react";
import { copyText } from "../lib/clipboard";
import { errorText } from "../lib/api";
import { openSharing, stopSharing, useRemote } from "../lib/remote";
import { useEditor } from "../lib/store";
import { Button, IconButton, trapTab } from "./ui";

/** Who has the page open: "iPhone", "iPhone and Mac", "3 devices". */
export function pagesText(pages: string[]) {
  if (pages.length === 0) return null;
  if (pages.length === 1) return pages[0];
  if (pages.length === 2) return `${pages[0]} and ${pages[1]}`;
  return `${pages.length} devices`;
}

/** In the top bar: opens the dialog, and while sharing says who is connected. */
export function ShareButton() {
  const sharing = useRemote((s) => !!s.sharing);
  const pages = useRemote((s) => s.pages);
  const label = "Edit from your phone or another computer";
  if (!sharing)
    return (
      <div className="bar flex rounded-full">
        <IconButton round label={label} aria-haspopup="dialog" onClick={() => void openSharing()}>
          <QrCode size={16} />
        </IconButton>
      </div>
    );
  return (
    <Button variant="bar" pill aria-haspopup="dialog" title={label} onClick={() => void openSharing()} className="max-w-[220px]">
      <QrCode size={15} className="shrink-0 text-accent" />
      <span className="truncate text-[12px]">{pagesText(pages) ?? "Sharing"}</span>
    </Button>
  );
}

export function ShareDialog() {
  const open = useRemote((s) => s.dialog);
  const sharing = useRemote((s) => s.sharing);
  const pages = useRemote((s) => s.pages);
  const error = useRemote((s) => s.error);
  const dialog = useRef<HTMLDivElement>(null);
  const close = () => useRemote.setState({ dialog: false });

  useEffect(() => {
    if (!open) return;
    const opener = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    dialog.current?.querySelector<HTMLElement>("[data-autofocus]")?.focus();
    const onKey = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      close();
    };
    window.addEventListener("keydown", onKey, true);
    return () => {
      window.removeEventListener("keydown", onKey, true);
      opener?.focus();
    };
  }, [open]);

  if (!open) return null;

  const copy = async () => {
    if (!sharing) return;
    try {
      await copyText(sharing.url, "The clipboard is not available here.");
      useEditor.getState().toast({ kind: "success", text: "Copied the link. It works on the same Wi-Fi." });
    } catch (e) {
      useEditor.getState().toast({ kind: "error", text: errorText(e) });
    }
  };
  const connected = pagesText(pages);
  return (
    <div data-exit className="fixed inset-0 z-[100] flex items-center justify-center scrim-in bg-black/45" onPointerDown={(e) => e.target === e.currentTarget && close()}>
      <div ref={dialog} role="dialog" aria-modal="true" aria-labelledby="share-title" onKeyDown={trapTab} className="overlay w-[560px] rounded-[20px] p-6 dialog-in">
        <div className="mb-5 flex items-start justify-between gap-3">
          <h2 id="share-title" className="text-[17px] leading-[22px] font-semibold tracking-[-0.025em]">
            Edit from your phone
          </h2>
          <IconButton label="Close" round className="-mt-1 -mr-2" onClick={close}>
            <X size={16} />
          </IconButton>
        </div>
        {error ? (
          <div className="flex flex-col items-start gap-3">
            <p role="alert" className="flex items-start gap-2 text-[13px] text-fg">
              <AlertCircle size={16} className="mt-px shrink-0 text-danger" />
              {error}
            </p>
            <Button pill data-autofocus onClick={() => void openSharing()}>
              Try again
            </Button>
          </div>
        ) : (
          <div className="flex gap-5">
            <div className="flex h-[196px] w-[196px] shrink-0 items-center justify-center rounded-xl bg-white p-2">
              {sharing ? (
                <img alt="QR code of the link" className="h-full w-full [image-rendering:pixelated]" src={`data:image/svg+xml;utf8,${encodeURIComponent(sharing.qr)}`} />
              ) : (
                <div className="skeleton h-full w-full rounded-lg" />
              )}
            </div>
            <div className="flex min-w-0 flex-1 flex-col gap-2">
              <p className="text-[13px] font-semibold text-fg">Scan the code with your phone's camera</p>
              <p className="text-[12px] text-muted">Or send the link to someone. They need to be on the same Wi-Fi as this computer.</p>
              <div className="mt-1 flex min-w-0 items-center gap-2">
                <span className="min-w-0 truncate rounded-md bg-white/[.055] px-2 py-1 text-[12px] text-fg tabular-nums">{sharing?.url ?? "…"}</span>
                <Button pill className="h-7 shrink-0 px-3 text-[12px]" disabled={!sharing} onClick={() => void copy()}>
                  Copy link
                </Button>
              </div>
              <p className="mt-1 flex items-center gap-1.5 text-[13px] font-medium text-fg" role="status">
                {connected ? <span className="h-1.5 w-1.5 shrink-0 rounded-full bg-ok" /> : null}
                {connected ? `${connected} connected` : <span className="font-normal text-muted">No one is connected yet</span>}
              </p>
              <p className="mt-auto text-[12px] text-muted">Anyone with the link can add videos to the open project, ask the AI to edit it and download what they export.</p>
            </div>
          </div>
        )}
        <div className="mt-6 flex justify-end gap-2">
          {sharing && (
            <Button pill onClick={() => void stopSharing()}>
              Stop sharing
            </Button>
          )}
          <Button pill variant="primary" data-autofocus={!error || undefined} onClick={close}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}

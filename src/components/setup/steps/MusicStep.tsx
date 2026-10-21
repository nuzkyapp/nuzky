import { useEffect } from "react";
import { AlertCircle, Check, Loader2 } from "lucide-react";
import { api, plainError } from "../../../lib/api";
import { checkYtdlp, useSetup } from "../../../lib/setup";
import { installYtdlp, useSounds } from "../../../lib/sounds";
import { useEditor } from "../../../lib/store";
import { AiLock, Button, LockReason, ProgressBar } from "../../ui";
import { StepActions } from "../StepActions";

const STOP = "Stop downloading yt-dlp";

/** The same download as the Music tab's; its failure, once the step is off screen, is a toast, unless Music shows it under its link. */
async function download() {
  await installYtdlp(false);
  const { installError: error, link } = useSounds.getState();
  const { open, current } = useSetup.getState();
  if (error && !error.startsWith("CANCELLED") && !(open && current === "music") && link.error !== error) useEditor.getState().toast({ kind: "error", text: plainError(error) });
}

export function MusicStep() {
  return (
    <AiLock>
      <Step />
    </AiLock>
  );
}

function Step() {
  const done = useSetup((s) => s.states.music === "done");
  const installing = useSounds((s) => s.installing);
  const progress = useSounds((s) => s.progress["yt-dlp"] ?? 0);
  const error = useSounds((s) => s.installError);
  const stopped = error?.startsWith("CANCELLED");

  useEffect(() => void checkYtdlp(), []);

  // Starting, stopping or ending the download removes the button that was pressed; the focus goes to what took its place.
  useEffect(() => {
    if (document.activeElement !== document.body) return;
    document.querySelector<HTMLElement>(installing ? `#setup-panel button[aria-label="${STOP}"]` : "#setup-panel footer button:not([data-sample])")?.focus();
  }, [installing, done]);

  return (
    <div className="flex flex-col gap-3 pt-3">
      <p className="text-[13px] leading-[18px] text-fg">
        Nuzky can add a song from a YouTube or YouTube Music link in Audio's Music tab.
        <span className="block text-muted">It needs yt-dlp, a free tool. Nuzky keeps its own copy up to date, so YouTube doesn't refuse an old one.</span>
      </p>
      {done ? (
        <p className="flex items-center gap-1.5 text-[12px] text-fg">
          <Check size={13} strokeWidth={3} className="shrink-0 text-ok" /> yt-dlp is downloaded.
        </p>
      ) : installing ? (
        <LockReason.Provider value={null}>
          <div className="flex flex-col gap-1.5" role="status">
            <div className="flex items-center gap-2 text-[12px]">
              <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
              <span className="min-w-0 flex-1 truncate text-fg">Downloading yt-dlp</span>
              {progress > 0 && <span className="tabular text-muted">{Math.round(progress * 100)}%</span>}
              <Button variant="ghost" className="h-7 px-2" aria-label={STOP} onClick={() => api.soundCancel("yt-dlp")}>
                Stop
              </Button>
            </div>
            <ProgressBar value={progress} label="Downloading yt-dlp" />
          </div>
        </LockReason.Provider>
      ) : stopped ? (
        <p className="text-[12px] text-muted" role="status">
          Stopped. Resume download goes on where it stopped.
        </p>
      ) : error ? (
        <p className="flex gap-1.5 text-[12px] text-danger" role="alert">
          <AlertCircle size={14} className="mt-px shrink-0" />
          {plainError(error)}
        </p>
      ) : null}
      {!done && !installing && (
        <StepActions>
          <Button pill variant="primary" onClick={download}>
            {stopped ? "Resume download" : error ? "Try again" : "Download yt-dlp"}
          </Button>
        </StepActions>
      )}
    </div>
  );
}

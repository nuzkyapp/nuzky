import { useEffect } from "react";
import { AlertCircle, AlertTriangle, Check, Loader2 } from "lucide-react";
import { api } from "../../../lib/api";
import { downloadModels, loadModels, useCover } from "../../../lib/cover";
import { useSetup } from "../../../lib/setup";
import { useEditor } from "../../../lib/store";
import { Button, ProgressBar } from "../../ui";
import { LogsButton } from "../../Feedback";
import { StepActions } from "../StepActions";

const STOP = "Stop downloading the face and person models";

/** Whether Download was pressed here, so the step shows only its own download's failure, not one of a cover pick. */
let asked = false;

/** Face and person models: what they are for and Download for all of them, so neither covers nor an agent ask later. */
export function VisionStep() {
  const models = useCover((s) => s.models);
  const job = useEditor((s) => {
    const id = useCover.getState().modelsJob;
    return id ? s.jobs[id] : undefined;
  });
  const failed = useCover((s) => s.failed);
  const done = useSetup((s) => s.states.vision === "done");
  const running = job?.status === "running";

  useEffect(() => void loadModels(), []);
  useEffect(() => {
    if (models?.downloaded && !done) useSetup.getState().finishStep("vision");
  }, [models?.downloaded, done]);
  // Starting or ending the download removes the button that was pressed; the focus goes to what took its place.
  useEffect(() => {
    if (document.activeElement !== document.body) return;
    document.querySelector<HTMLElement>(running ? `#setup-panel button[aria-label="${STOP}"]` : "#setup-panel footer button")?.focus();
  }, [running]);

  if (!models) return null;
  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex flex-col gap-3 pt-3 text-[13px] text-fg/85">
        <p>
          These models find faces and the person in your video, on this computer. Nuzky uses them to pick a cover frame, put text behind the person, blur the background and follow a
          face when it reframes a video to vertical. Your AI uses them too.
        </p>
        {models.unavailable ? (
          <p className="flex gap-1.5 text-[12px] text-fg" role="alert">
            <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
            They can't run on this computer, so there is nothing to download. Everything else works.
          </p>
        ) : models.downloaded ? (
          <p className="flex items-center gap-1.5 text-[12px] text-fg">
            <Check size={12} strokeWidth={3} className="text-ok" /> Downloaded
          </p>
        ) : (
          <p className="tabular text-[12px] text-muted">A one-time download of {models.sizeMb} MB.</p>
        )}
      </div>
      <div className="flex-1" />
      {running ? (
        <div className="mt-3 flex flex-col gap-1.5" role="status">
          <div className="flex items-center gap-2 text-[12px]">
            <Loader2 size={13} className="shrink-0 animate-spin text-accent" />
            <span className="min-w-0 flex-1 truncate text-fg">Downloading face and person models</span>
            {job.progress > 0 && <span className="tabular text-muted">{Math.round(job.progress * 100)}%</span>}
            <Button variant="ghost" className="h-7 px-2" aria-label={STOP} onClick={() => void api.cancelJob(job.id)}>
              Stop
            </Button>
          </div>
          <ProgressBar value={job.progress} label="Downloading the face and person models" />
        </div>
      ) : (
        asked &&
        failed && (
          <div className="mt-3 flex gap-1.5 text-[12px] text-danger" role="alert">
            <AlertCircle size={14} className="mt-px shrink-0" />
            <div className="flex min-w-0 flex-1 flex-col items-start gap-1">
              <p>{failed}</p>
              <LogsButton />
            </div>
          </div>
        )
      )}
      {!running && !models.downloaded && !models.unavailable && (
        <StepActions>
          <Button
            pill
            variant="primary"
            onClick={() => {
              asked = true;
              void downloadModels();
            }}
          >
            {asked && failed ? "Try again" : `Download ${models.sizeMb} MB`}
          </Button>
        </StepActions>
      )}
    </div>
  );
}

import { useEffect, useRef, useState } from "react";
import { api, errorText, plainError } from "../lib/api";
import { usePreview } from "../lib/preview";
import { useEditor } from "../lib/store";
import type { ModelKind } from "../lib/types";
import { Button, trapTab } from "./ui";

export const MODEL_QUESTION: Record<ModelKind, { title: string; text: (mb: number) => string }> = {
  speech: {
    title: "Download the speech model?",
    text: (mb) =>
      `The AI needs to know what is said in your video. Nuzky recognises speech on this computer with a speech model, a one-time download of ${mb} MB. Once it is downloaded, the AI goes on by itself; with Not now, it goes on without a transcript.`,
  },
  vision: {
    title: "Download the face and person models?",
    text: (mb) =>
      `The AI needs them to find faces and cut out the person, for covers, reframing and backgrounds. They run on this computer, a one-time download of ${mb} MB. Once they are downloaded, the AI goes on by itself; with Not now, it goes on without them.`,
  },
};

/** The label of the download of the face and person models an agent asked for (src-tauri/src/model_question.rs). */
export const AGENT_VISION_MODELS = "Face and person models";

const question = (kind: ModelKind) => (kind === "speech" ? "speechModelQuestion" : "visionModelQuestion");

/** Answers what an agent's job waits for. */
export function answerModelQuestion(kind: ModelKind, download: boolean) {
  const asked = useEditor.getState()[question(kind)];
  if (!asked) return;
  useEditor.setState({ [question(kind)]: null, ...(kind === "vision" && download ? { visionModelJob: `vision-models:${asked.id}` } : {}) });
  api.answerModelQuestion(asked.id, download).catch((e) => useEditor.getState().toast({ kind: "error", text: plainError(errorText(e)) }));
}

/** Asks whether to download a model an agent needs; the agent waits for the answer. The editor's AI panel asks for the face and person models itself while it is on screen. */
export function ModelQuestionDialog({ kind }: { kind: ModelKind }) {
  // The editor stays behind the home screen with its panel, and the full screen preview hides the panel.
  const full = usePreview((s) => s.full);
  const asked = useEditor((s) => (kind === "vision" && s.aiChats > 0 && s.view === "editor" && !full ? null : s[question(kind)]));
  // What it asked stays while it closes, so it fades out whole.
  const [shown, setShown] = useState(asked);
  if (asked && asked !== shown) setShown(asked);
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    // Closing, also when the agent no longer waits, gives focus back to where it was.
    if (!asked) return void (d.open && d.close());
    if (d.open) return;
    d.showModal();
    // It opens unasked, so the dialog takes focus rather than a button: a Space typed meanwhile starts nothing.
    d.focus();
  }, [asked]);
  const answer = (download: boolean) => void answerModelQuestion(kind, download);
  return (
    <dialog
      data-exit
      ref={dialog}
      tabIndex={-1}
      aria-labelledby={`${kind}-model-title`}
      aria-describedby={`${kind}-model-text`}
      onCancel={(e) => {
        e.preventDefault();
        answer(false);
      }}
      onKeyDown={(e) => {
        // The editor's keys (Space plays, Delete removes) wait until it is answered.
        e.stopPropagation();
        if (e.key === "Enter" && e.target === e.currentTarget) answer(true);
        trapTab(e);
      }}
      className="dialog-in overlay fixed inset-0 m-auto w-[440px] rounded-[20px] p-6 text-fg outline-none backdrop:bg-black/45"
    >
      {shown && (
        <>
          <h2 id={`${kind}-model-title`} className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            {MODEL_QUESTION[kind].title}
          </h2>
          <p id={`${kind}-model-text`} className="mt-2 text-[13px] text-fg/85">
            {MODEL_QUESTION[kind].text(shown.sizeMb)}
          </p>
          <div className="mt-6 flex justify-end gap-2">
            <Button pill onClick={() => answer(false)}>
              Not now
            </Button>
            <Button pill variant="primary" onClick={() => answer(true)}>
              Download
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}

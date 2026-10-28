import { useEffect, useRef, useState } from "react";
import { CircleMinus, Sparkles, TriangleAlert } from "lucide-react";
import { send, sendBlocked, useAgent } from "../../lib/agent";
import { FINISH_DRAFT, leftOutLine, plural, useCapCut } from "../../lib/capcut";
import { useDock } from "../../lib/dock";
import { isCaptionTrack, useEditor } from "../../lib/store";
import type { Project } from "../../lib/types";
import { Button, trapTab } from "../ui";

/** What came over from the draft, as one sentence: "31 clips, 191 captions and 2 zooms or moves". */
function cameOver(project: Project): string {
  const tracks = project.tracks;
  const clips = tracks.filter((t) => t.kind === "video").flatMap((t) => t.clips).length;
  const sounds = tracks.filter((t) => t.kind === "audio").flatMap((t) => t.clips).length;
  const captions = tracks.filter(isCaptionTrack).flatMap((t) => t.clips).length;
  const titles = tracks.filter((t) => t.kind === "text" && !isCaptionTrack(t)).flatMap((t) => t.clips).length;
  const moves = tracks.flatMap((t) => t.clips).filter((c) => c.keyframes.length > 0).length;
  const parts = [
    clips && plural(clips, "clip"),
    sounds && plural(sounds, "sound"),
    captions && plural(captions, "caption"),
    titles && plural(titles, "title"),
    moves && plural(moves, "zoom or move", "zooms or moves"),
  ].filter((p): p is string => !!p);
  if (parts.length === 0) return "Nothing came over.";
  const list = parts.length === 1 ? parts[0] : `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
  return `${list[0].toUpperCase()}${list.slice(1)} came over. These did not:`;
}

/** Once a draft is open: what came over and, in one list, what did not, with the way to let the AI finish it. */
export function OpenedDraftDialog() {
  const opened = useCapCut((s) => s.opened);
  const snap = useEditor((s) => s.snap);
  const project = snap && snap.path === opened ? snap.project : null;
  const imported = project?.imported;
  // What it said stays while it closes, so it fades out whole.
  const [shown, setShown] = useState(project && imported ? { project, imported } : null);
  if (project && imported && (project !== shown?.project || imported !== shown?.imported)) setShown({ project, imported });
  const dialog = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = dialog.current;
    if (!d) return;
    if (!imported) return void (d.open && d.close());
    if (d.open) return;
    d.showModal();
    // Done, not Finish with AI: an Enter pressed meanwhile must not send anything to the AI.
    d.querySelector<HTMLElement>("[data-autofocus]")?.focus();
  }, [imported]);
  // Another project opened meanwhile has nothing to say here.
  useEffect(() => {
    if (opened && snap && snap.path !== opened) useCapCut.setState({ opened: null });
  }, [opened, snap]);

  const close = () => useCapCut.setState({ opened: null });
  const finish = () => {
    close();
    useDock.setState({ open: true });
    // Make a thumbnail waiting in the field would turn the message into a thumbnail request.
    useAgent.setState({ thumbnail: false });
    if (sendBlocked()) useAgent.setState({ draft: FINISH_DRAFT });
    else void send(FINISH_DRAFT, "project");
  };
  return (
    <dialog
      data-exit
      ref={dialog}
      aria-labelledby="opened-draft-title"
      onCancel={(e) => {
        e.preventDefault();
        close();
      }}
      onKeyDown={(e) => {
        e.stopPropagation();
        trapTab(e);
      }}
      className="dialog-in overlay fixed inset-0 m-auto w-[480px] max-w-[calc(100vw-48px)] rounded-[20px] p-6 text-fg outline-none backdrop:bg-black/45"
    >
      {shown && (
        <>
          <h2 id="opened-draft-title" className="text-[17px] font-semibold leading-[22px] tracking-[-0.025em]">
            Opened “{shown.project.name}” from {shown.imported.editor}
          </h2>
          <p className="mt-2 text-[13px] text-muted">{cameOver(shown.project)}</p>
          <ul aria-label="Did not come over" className="mt-3.5 max-h-[min(320px,calc(100vh-280px))] overflow-y-auto rounded-xl bg-white/[.055] px-3 py-1 shadow-[inset_0_0_0_1px_rgba(255,255,255,.06)]">
            {shown.imported.leftOut.map((line) => {
              const { what, count, missing } = leftOutLine(line);
              return (
                <li key={line} className="flex items-start gap-2.5 border-t border-white/[.06] py-2 text-[13px] leading-[18px] first:border-t-0">
                  {missing ? <TriangleAlert size={15} className="mt-px shrink-0 text-warn" /> : <CircleMinus size={15} className="mt-px shrink-0 text-muted" />}
                  <span className="min-w-0 flex-1 break-words">{what}</span>
                  {count && <span className="tabular shrink-0 text-[12px] text-muted">{count}</span>}
                </li>
              );
            })}
          </ul>
          <div className="mt-5 flex items-center justify-end gap-2">
            <span className="mr-auto text-[12px] text-muted">The draft in {shown.imported.editor} stays as it was.</span>
            <Button pill onClick={finish}>
              <Sparkles size={15} /> Finish with AI
            </Button>
            <Button pill variant="primary" data-autofocus onClick={close}>
              Done
            </Button>
          </div>
        </>
      )}
    </dialog>
  );
}

import { useMemo, type ReactNode } from "react";
import { Captions, Check, ChevronDown, Film, ImagePlus, Music, Scissors, ZoomIn, type LucideIcon } from "lucide-react";
import { COVER_FORMATS, openCover, thumbnailOf } from "../../lib/cover";
import { CAPTION_STYLES, formatLabel, isLook } from "../../lib/presets";
import { useSounds } from "../../lib/sounds";
import { speechBlocker, useTranscriptView } from "../../lib/speech";
import { isCaptionTrack, projectDuration, useEditor, type PanelTab } from "../../lib/store";
import { formatLength } from "../../lib/time";
import type { Project, TranscriptView } from "../../lib/types";
import { AiLock, Button, ColorInput, Section, Segmented, Slider, lockedProps, tabIds, useLockReason } from "../ui";
import { InspectorHeader } from "./Header";

const DEFAULT_BLUR = 0.5;

const count = (n: number, what: string) => `${n} ${what}${n === 1 ? "" : "s"}`;

let waiting = 0;

/**
 * Focuses the control once it shows: a library tab mounts on the next render, the Transcript's buttons only once
 * the transcript arrives. Gives up when another step is pressed or the user moves the focus meanwhile.
 */
function focusWhenShown(selector: string) {
  const mine = ++waiting;
  const from = document.activeElement;
  const until = performance.now() + 3000;
  const look = () => {
    const now = document.activeElement;
    if (mine !== waiting || (now !== from && now !== document.body)) return;
    const target = document.querySelector<HTMLElement>(selector);
    if (target) target.focus();
    else if (performance.now() < until) requestAnimationFrame(look);
  };
  requestAnimationFrame(look);
}

/** Each control exists once: a step opens the library tab that has it and focuses it there. */
function reach(tab: PanelTab, selector = `#${tabIds("library", tab).tab}`) {
  useEditor.setState({ panelTab: tab });
  focusWhenShown(selector);
}

function Step({ icon: Icon, done, title, detail, children }: { icon: LucideIcon; done?: boolean; title: string; detail?: string; children?: ReactNode }) {
  return (
    <li className="flex min-h-10 items-center gap-2.5">
      {done ? <Check size={16} className="shrink-0 text-ok" aria-hidden /> : <Icon size={16} className="shrink-0 text-muted" aria-hidden />}
      <div className="flex min-w-0 flex-1 flex-col leading-[17px]">
        <span className="text-fg">
          {title}
          {done && <span className="sr-only">, done</span>}
        </span>
        {detail && <span className="tabular truncate text-[12px] text-muted">{detail}</span>}
      </div>
      {children}
    </li>
  );
}

const STEP_BUTTON = "h-7 shrink-0 px-2.5 text-[12px]";

function SpeechStep({ project, view, error }: { project: Project; view: TranscriptView | null; error: string | null }) {
  const blocker = speechBlocker(project);
  if (blocker) return <Step icon={Scissors} title="Speech" detail={blocker} />;
  if (error)
    return (
      <Step icon={Scissors} title="Speech" detail={error}>
        <Button className={STEP_BUTTON} aria-label="Open Transcript" onClick={() => reach("transcript")}>
          Open
        </Button>
      </Step>
    );
  if (!view) return <Step icon={Scissors} title="Speech" detail="Checking…" />;
  if (view.untranscribed.length > 0)
    return (
      <Step icon={Scissors} title="Speech" detail={view.words.length ? "Partly transcribed" : "Not transcribed"}>
        <Button variant="primary" className={STEP_BUTTON} title="Open Transcript to recognise what is said" onClick={() => reach("transcript", "[data-transcribe]")}>
          Transcribe
        </Button>
      </Step>
    );
  if (view.words.length === 0) return <Step icon={Scissors} title="Speech" detail="No words recognised" />;
  const left = [
    view.retakes && count(view.retakes, "retake"),
    view.fillers && count(view.fillers, "filler word"),
    view.hesitations.length && count(view.hesitations.length, "hesitation"),
    view.pauses.length && count(view.pauses.length, "pause"),
  ].filter(Boolean);
  if (!left.length) return <Step icon={Scissors} done title="Speech" detail="Nothing left to cut" />;
  return (
    <Step icon={Scissors} title="Speech" detail={left.join(", ")}>
      <Button variant="primary" className={STEP_BUTTON} title="Open Transcript to remove retakes and pauses" onClick={() => reach("transcript", "[data-remove-retakes]")}>
        Clean up
      </Button>
    </Step>
  );
}

function captionsOf(project: Project) {
  const clips = project.tracks.filter(isCaptionTrack).flatMap((t) => t.clips);
  const looks = clips.map(({ content: c, animIn, animOut }) => (c.type === "text" ? CAPTION_STYLES.find((p) => isLook(p, c.style, animIn, animOut)) : undefined));
  const style = looks.every((l) => l === looks[0]) ? looks[0]?.name : undefined;
  return { count: clips.length, style };
}

function soundsOf(project: Project) {
  return project.tracks
    .filter((t) => t.kind === "audio")
    .flatMap((t) => t.clips)
    .flatMap((c) => {
      const content = c.content;
      const asset = content.type === "media" ? project.assets.find((a) => a.id === content.assetId) : undefined;
      return asset?.kind === "audio" ? [asset.name] : [];
    });
}

/** What the video still needs, each step with the place that does it. */
function ProjectStatus({ project }: { project: Project }) {
  const captions = useMemo(() => captionsOf(project), [project]);
  const sounds = useMemo(() => soundsOf(project), [project]);
  const covers = COVER_FORMATS.filter((f) => thumbnailOf(project, f.id));
  const { view, error } = useTranscriptView();
  const empty = projectDuration(project) <= 0;
  const openAudio = (tab: "project" | "music", selector?: string) => {
    useSounds.setState({ tab });
    reach("audio", selector);
  };
  return (
    <Section>
      <ul aria-label="Project status" className="-my-1 flex flex-col">
        <SpeechStep project={project} view={view} error={error} />
        <Step icon={Captions} done={captions.count > 0} title="Captions" detail={captions.count ? count(captions.count, "caption") + (captions.style ? ` · ${captions.style}` : "") : "None"}>
          <Button
            className={STEP_BUTTON}
            aria-label={captions.count ? "Edit captions" : "Add captions"}
            title={captions.count ? "Open Captions" : "Open Captions to generate them"}
            onClick={() => (captions.count ? reach("captions") : reach("captions", "[data-generate-captions]"))}
          >
            {captions.count ? "Edit" : "Add"}
          </Button>
        </Step>
        <Step icon={ZoomIn} title="Zooms" detail={view?.words.length ? undefined : "Needs a transcript"}>
          <Button
            className={STEP_BUTTON}
            aria-label="Suggest zooms"
            disabled={!view?.words.length}
            disabledReason="Transcribe the timeline first"
            title="Open Transcript to zoom in on the sentences said with emphasis"
            onClick={() => reach("transcript", "[data-suggest-zooms]")}
          >
            Suggest
          </Button>
        </Step>
        <Step icon={Music} done={sounds.length > 0} title="Music" detail={sounds.length === 1 ? sounds[0] : sounds.length ? count(sounds.length, "sound") : "None"}>
          {sounds.length ? (
            <Button className={STEP_BUTTON} aria-label="Edit music" title="Open the sounds in the project" onClick={() => openAudio("project")}>
              Edit
            </Button>
          ) : (
            <Button className={STEP_BUTTON} aria-label="Add music" title="Open free music in Audio" onClick={() => openAudio("music", 'input[placeholder="Search free music"]')}>
              Add
            </Button>
          )}
        </Step>
        <Step icon={ImagePlus} done={covers.length > 0} title="Cover" detail={covers.length ? covers.map((f) => f.short).join(" and ") : "None"}>
          <Button
            className={STEP_BUTTON}
            disabled={empty}
            disabledReason="Add a clip to the timeline first"
            aria-label={covers.length ? "Edit the cover" : "Make a cover"}
            title={covers.length ? "Open the cover editor" : "Open the cover editor to make one"}
            onClick={() => {
              openCover(covers[0]?.id);
              focusWhenShown(covers.length ? '[role=group][aria-label="Cover format"] [aria-pressed=true]' : "[data-make-cover-ai]");
            }}
          >
            {covers.length ? "Edit" : "Make"}
          </Button>
        </Step>
      </ul>
    </Section>
  );
}

function ProjectSettings() {
  const lock = useLockReason();
  const canvas = useEditor((s) => s.snap!.project.canvas);
  const edit = useEditor((s) => s.edit);
  // Size comes from the latest confirmed canvas, so a Ratio change made a moment ago is kept.
  const setCanvas = (patch: { background?: string; backgroundBlur?: number }, key?: string) =>
    edit((p) => ({ type: "setCanvas", width: p.canvas.width, height: p.canvas.height, ...patch }), key);
  const blur = canvas.backgroundBlur > 0;
  return (
    <>
      <Section title="Format">
        {/* Opens Ratio under the preview, which stays the one control for the format. */}
        <button
          type="button"
          aria-haspopup="menu"
          title="Change the canvas ratio"
          onClick={() => useEditor.setState({ ratioOpen: true })}
          {...lockedProps(lock)}
          className="tabular -mx-1.5 -my-1 inline-flex h-6 items-center gap-1 self-start rounded-md px-1.5 text-[12px] text-fg transition-colors duration-[120ms] ease-out hover:bg-white/[.08] aria-disabled:cursor-not-allowed aria-disabled:opacity-40 aria-disabled:hover:bg-transparent"
        >
          {formatLabel(canvas.width, canvas.height)} · {canvas.width}×{canvas.height}
          <ChevronDown size={13} className="text-muted" />
        </button>
      </Section>
      <Section title="Background">
        <Segmented
          label="Background"
          value={blur ? "blur" : "color"}
          onChange={(v) => setCanvas({ backgroundBlur: v === "blur" ? DEFAULT_BLUR : 0 })}
          options={[
            { id: "color", label: "Color", title: "Fill empty space with a color" },
            { id: "blur", label: "Blur", title: "Fill empty space with a blurred copy of the video" },
          ]}
        />
        {blur ? (
          <Slider
            label="Strength"
            value={canvas.backgroundBlur * 100}
            min={5}
            max={100}
            step={1}
            unit="%"
            format={(v) => String(Math.round(v))}
            onChange={(v) => setCanvas({ backgroundBlur: v / 100 }, "canvas-blur")}
          />
        ) : (
          <ColorInput label="Color" value={canvas.background} onChange={(v) => setCanvas({ background: v }, "canvas-bg")} />
        )}
      </Section>
    </>
  );
}

/** The inspector with nothing selected: what the video still needs, then the project's format and background. */
export function ProjectSection() {
  const project = useEditor((s) => s.snap!.project);
  const { fps } = project.canvas;
  return (
    <>
      <InspectorHeader icon={Film} title="Project" detail={`${formatLength(projectDuration(project))} · ${fps} fps`} />
      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* The steps only open the tab that does the work; that tab is locked while the AI edits. */}
        <ProjectStatus project={project} />
        <AiLock>
          <ProjectSettings />
        </AiLock>
      </div>
    </>
  );
}

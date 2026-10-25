import { useEffect, useRef, useState, type ReactNode } from "react";
import { AlertTriangle, Check, Scissors } from "lucide-react";
import { RHYTHMS, cutToBeat, loadBeats, useBeats } from "../../lib/beats";
import { api, errorText, plainError } from "../../lib/api";
import { useEditor } from "../../lib/store";
import { US, formatTime } from "../../lib/time";
import type { Asset, BeatPlans, Clip, Rhythm, RhythmPlan } from "../../lib/types";
import { Button, Checkbox, ProgressBar, Section, lockedProps, useLockReason } from "../ui";

const LINE = "text-[12px] leading-[17px] text-muted";

function Warning({ children }: { children: string }) {
  return (
    <p className={`flex items-start gap-1.5 ${LINE}`}>
      <AlertTriangle size={14} className="mt-px shrink-0 text-warn" aria-hidden />
      {children}
    </p>
  );
}

/** A file's name in a sentence: "City walk", not "City walk.m4a". */
const songName = (name: string) => name.replace(/\.[a-z0-9]{2,4}$/i, "");
const plural = (n: number, one: string, many = `${one}s`) => `${n} ${n === 1 ? one : many}`;
const seconds = (us: number) => `${(us / US).toFixed(1)} s`;
/** Where the beat comes in after an intro worth naming; a beat from the first second has none. */
const intro = (beatFromUs: number | null) => (beatFromUs !== null && beatFromUs >= US ? beatFromUs : null);

function footage(plans: BeatPlans) {
  const { videos, pictures } = plans;
  if (pictures === 0) return `your ${plural(videos, "video")}`;
  if (videos === 0) return `your ${plural(pictures, "picture")}`;
  return `your ${videos + pictures} videos and pictures`;
}

/** Why a rhythm cuts the same as an earlier one, in one line, or what it does. */
function describe(plan: RhythmPlan, plans: BeatPlans, song: string) {
  const does = RHYTHMS.find((r) => r.id === plan.rhythm)!.does;
  if (!plan.sameAs) return does;
  const other = RHYTHMS.find((r) => r.id === plan.sameAs)!.label;
  if (plan.rhythm === "builds" && plans.buildsDropUs === null) {
    const at = plans.beatFromUs;
    const why = plans.song.drops.some((d) => d.beatStarts) && at !== null ? `${song} drops where its beat starts, at ${formatTime(at, false)}` : `${song} has no drop${intro(at) ? " after its intro" : ""}`;
    return `${does} ${why}, so here this is the same as ${other}.`;
  }
  return `${does} Here it cuts the same as ${other}.`;
}

/**
 * What cutting in this rhythm will do, before it is done: one line, its warnings and the Cut row first, so they show
 * in the smallest window, then what the rhythm does and the rest on demand.
 */
function Consequences({ plan, plans, song, keepSound, children }: { plan: RhythmPlan; plans: BeatPlans; song: string; keepSound: boolean; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const shorter = plans.songEndUs < plans.footageEndUs;
  const line = [
    `${plural(plan.clips, "clip")} from ${footage(plans)}`,
    keepSound ? "their own sound kept" : "their sound muted",
    plan.captionsRemoved > 0 && `${plural(plan.captionsRemoved, "speech caption")} removed`,
    plan.repeated > 0 && `${plural(plan.repeated, "clip")} ${plan.repeated === 1 ? "repeats" : "repeat"} footage`,
  ]
    .filter(Boolean)
    .join(" · ");
  const until = intro(plans.beatFromUs);
  const before = until !== null ? `The first ${seconds(until)}, before the beat,` : null;
  const details = [
    plan.repeated > 0 && `Your videos run out, so ${plan.repeated} of the ${plan.clips} clips show footage again. More videos avoid it.`,
    before && (plan.introCut ? `${before} change shot on bar lines, as no video is that long.` : `${before} stay one shot.`),
    plan.captionsRemoved > 0 ? "The captions of the old speech go with it. Titles and stickers keep their time." : "Titles and stickers keep their time.",
    !shorter && `Ends on the bar at ${formatTime(plan.endUs)}.${plan.endUs < plans.songEndUs ? ` ${song} fades out over it.` : ""}`,
    plans.coverFrame && "Your cover shows a frame of the video. Choose it again after cutting.",
  ].filter((d): d is string => !!d);
  return (
    <>
      {plan.repeated > 0 ? <Warning>{line}</Warning> : <p className={`tabular ${LINE}`}>{line}</p>}
      {shorter && <Warning>{`${song} is shorter. The video ends with it on the bar at ${formatTime(plan.endUs)}, ${seconds(plans.footageEndUs - plan.endUs)} shorter.`}</Warning>}
      {plans.videos + plans.pictures === 1 && plan.clips > 1 && <Warning>Only one video, so each clip goes on where the last stopped and the cuts hardly show.</Warning>}
      {children}
      <p className={LINE}>{describe(plan, plans, song)}</p>
      <button type="button" aria-expanded={open} onClick={() => setOpen(!open)} className="-my-1 self-start rounded-md py-0.5 text-[12px] font-medium text-accent hover:underline">
        {open ? "Fewer details" : "Details"}
      </button>
      {open && (
        <ul className={`-mt-1 flex list-disc flex-col gap-1 pl-4 ${LINE}`}>
          {details.map((d) => (
            <li key={d}>{d}</li>
          ))}
        </ul>
      )}
    </>
  );
}

/**
 * Beat cuts for the song on a music track: its beat, found as a job, three rhythms with what each would cut, and the
 * cut itself as one undo step. Agents do the same with cut_to_beats.
 */
export function BeatsSection({ clip, asset }: { clip: Clip; asset: Asset }) {
  const lock = useLockReason();
  const revision = useEditor((s) => s.snap?.revision);
  const job = useEditor((s) => s.jobs[`beats:${asset.id}`]);
  // While the song's sound is still being prepared, finding waits for it; that is most of the wait.
  const preparing = useEditor((s) => (s.jobs[`audio:${asset.id}`]?.status === "running" ? s.jobs[`audio:${asset.id}`].progress : null));
  const plans = useBeats((s) => s.plans[clip.id]);
  const error = useBeats((s) => s.errors[clip.id]);
  const chosen = useBeats((s) => s.chosen[clip.id]);
  const made = useBeats((s) => s.made[clip.id]);
  const keepSound = useBeats((s) => s.keepSound);
  const cutting = useBeats((s) => s.cutting);
  const reveal = useBeats((s) => s.reveal === clip.id);
  const section = useRef<HTMLElement>(null);
  const song = songName(asset.name);
  const finding = job?.status === "running";
  // Finding starts by itself until it failed or was stopped; then it waits for Try again.
  const find = !job || job.status === "done";

  useEffect(() => {
    void loadBeats(clip.id, asset.id, find);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- the plans follow the project; the job's end loads them itself
  }, [clip.id, asset.id, revision]);

  const ready = !!plans && plans.clipId === clip.id;
  useEffect(() => {
    if (!reveal) return;
    section.current?.scrollIntoView({ block: "start" });
    if (!ready) return;
    const row = section.current?.querySelector<HTMLElement>("[data-rhythm][aria-pressed=true]") ?? section.current?.querySelector<HTMLElement>("[data-rhythm]");
    // Only when the focus went nowhere else meanwhile, as when Project status's button left with the selection.
    const focus = document.activeElement;
    if (!focus || focus === document.body || section.current?.closest("aside")?.contains(focus)) row?.focus({ preventScroll: true });
    // In the smallest window the section is taller than the inspector: the Cut row stays in view, the header may not.
    section.current?.querySelector("[data-cut-to-beat]")?.scrollIntoView({ block: "nearest" });
    useBeats.setState({ reveal: null });
  }, [reveal, ready]);

  const current = plans?.current ?? null;
  // A cut picked from a row: the focus moves to the row now checked, so only one row looks chosen.
  const currentRhythm = current?.rhythm;
  useEffect(() => {
    const focus = document.activeElement;
    if (currentRhythm && focus instanceof HTMLElement && focus.dataset.rhythm && section.current?.contains(focus))
      section.current.querySelector<HTMLElement>(`[data-rhythm="${currentRhythm}"]`)?.focus();
  }, [currentRhythm]);
  const usable = plans?.rhythms.filter((r) => !r.error) ?? [];
  const pick: Rhythm | undefined = current?.rhythm ?? (usable.some((r) => r.rhythm === chosen) ? chosen : (usable.find((r) => r.rhythm === "builds") ?? usable[0])?.rhythm);
  const plan = plans?.rhythms.find((r) => r.rhythm === pick);
  const header = plans?.steady ? `${plans.bpm} BPM${intro(plans.beatFromUs) !== null ? ` · beat from ${formatTime(plans.beatFromUs!, false)}` : ""}` : undefined;
  const choose = (r: RhythmPlan) => {
    if (cutting || r.error) return;
    // After a cut, picking is cutting: it replaces that cut, or recuts the clips as they are.
    if (current) {
      if (!(current.newest && r.rhythm === current.rhythm)) void cutToBeat(clip.id, asset.id, song, r.rhythm);
    } else useBeats.setState((s) => ({ chosen: { ...s.chosen, [clip.id]: r.rhythm } }));
  };
  const retry = () => {
    useEditor.setState((s) => {
      const jobs = { ...s.jobs };
      delete jobs[`beats:${asset.id}`];
      return { jobs };
    });
    useBeats.setState((s) => {
      const errors = { ...s.errors };
      delete errors[clip.id];
      return { errors };
    });
    void api.findBeats(clip.id, useEditor.getState().snap?.sessionEpoch).catch((e) => useBeats.setState((s) => ({ errors: { ...s.errors, [clip.id]: plainError(errorText(e)) } })));
  };

  const cutRow = plan && (
    <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-2">
      <Checkbox label="Keep the videos’ own sound" checked={keepSound} onChange={(v) => useBeats.setState({ keepSound: v })} />
      <Button
        variant="primary"
        className="ml-auto"
        data-cut-to-beat
        disabled={cutting || !!plan.error}
        disabledReason={plan.error ?? "Cutting…"}
        onClick={() => pick && void cutToBeat(clip.id, asset.id, song, pick)}
      >
        <Scissors size={14} /> {cutting ? "Cutting…" : "Cut to the beat"}
      </Button>
    </div>
  );

  let body;
  if (finding) {
    const doing = `Finding the beat in ${song}`;
    const progress = job.progress > 0 ? job.progress : (preparing ?? 0) * 0.95;
    body = (
      <div className="flex flex-col gap-1.5" role="status">
        <span className={`tabular ${LINE}`}>
          {doing}
          {progress > 0 && ` · ${Math.round(progress * 100)}%`}
        </span>
        <ProgressBar value={progress} label={doing} />
      </div>
    );
  } else if (job?.status === "failed" || job?.status === "cancelled" || (error && !ready)) {
    body = (
      <>
        {job?.status === "cancelled" ? (
          <p className={LINE}>Stopped before the beat was found.</p>
        ) : (
          <p role="status" className={`flex items-start gap-1.5 ${LINE}`}>
            <AlertTriangle size={14} className="mt-px shrink-0 text-danger" aria-hidden />
            {job?.status === "failed" ? `The beat could not be found: ${plainError(job.message ?? "").replace(/\.$/, "")}.` : error}
          </p>
        )}
        <Button className="self-start" onClick={retry}>
          {job?.status === "cancelled" ? "Find the beat" : "Try again"}
        </Button>
      </>
    );
  } else if (!ready) {
    body = <p className={LINE}>Finding the beat…</p>;
  } else if (!plans.steady) {
    body = (
      <>
        <Warning>{`No steady beat in ${song}, so cuts would have nothing to land on.`}</Warning>
        <p className={LINE}>Add a song with drums from Audio › Music and cut to that.</p>
      </>
    );
  } else if (usable.length === 0) {
    const reasons = [...new Set(plans.rhythms.map((r) => r.error))];
    body = reasons.map((r) => <Warning key={r}>{r ?? ""}</Warning>);
  } else {
    body = (
      <>
        <div role="group" aria-label="Rhythm" className="flex flex-col gap-1.5">
          {plans.rhythms.map((r) => {
            const meta = RHYTHMS.find((m) => m.id === r.rhythm)!;
            const on = r.rhythm === pick;
            const isCurrent = current?.rhythm === r.rhythm;
            const count = r.error ? "—" : plural(r.clips, "clip");
            const inert = !!lock || !!r.error || cutting;
            return (
              <button
                key={r.rhythm}
                type="button"
                data-rhythm={r.rhythm}
                aria-pressed={on}
                aria-label={`${meta.label}${isCurrent ? ", current" : ""}, ${count}`}
                title={r.error ?? (current && !isCurrent ? `Recut in ${meta.label}` : meta.does)}
                onClick={() => choose(r)}
                {...lockedProps(lock)}
                aria-disabled={inert || undefined}
                className={`flex h-8 min-w-0 items-center gap-2 rounded-lg px-2.5 text-left text-[13px] transition-colors duration-[120ms] ease-out aria-disabled:cursor-not-allowed aria-disabled:opacity-40 ${
                  on ? "bg-accent/15 font-medium text-fg shadow-[inset_0_0_0_1px_var(--color-accent)]" : `bg-white/[.055] text-fg shadow-[inset_0_0_0_1px_rgb(255_255_255/.06)] ${inert ? "" : "hover:bg-white/[.09]"}`
                }`}
              >
                <span className="flex w-3.5 shrink-0 justify-center">{on && <Check size={14} className="text-accent" aria-hidden />}</span>
                <span className="truncate">{meta.label}</span>
                {isCurrent && <span className="shrink-0 text-[11px] font-normal text-muted">Current</span>}
                <span className="tabular ml-auto shrink-0 text-[12px] font-normal text-muted">{count}</span>
              </button>
            );
          })}
        </div>
        {current ? (
          <>
            <p className={LINE}>
              {current.newest ? "Picking another rhythm replaces this cut. It stays one Undo." : "You edited the video after this cut. Picking a rhythm now recuts the clips as they are."}
            </p>
            {made && made.rhythm === current.rhythm && (
              <p className={`tabular ${LINE}`}>
                {`${plural(made.clips, "clip")} from ${footage(plans)}, ${made.keepSound ? "their own sound kept" : "their sound muted"}`}
                {made.captionsRemoved > 0 && `. Removed ${plural(made.captionsRemoved, "speech caption")}`}
              </p>
            )}
            {made && made.rhythm === current.rhythm && made.repeated > 0 && <Warning>{`Your videos ran out, so ${plural(made.repeated, "clip")} ${made.repeated === 1 ? "shows" : "show"} footage again.`}</Warning>}
          </>
        ) : (
          plan && (
            <>
              {plan.error ? (
                <>
                  <Warning>{plan.error}</Warning>
                  {cutRow}
                </>
              ) : (
                <Consequences plan={plan} plans={plans} song={song} keepSound={keepSound}>
                  {cutRow}
                </Consequences>
              )}
            </>
          )
        )}
      </>
    );
  }

  return (
    <Section ref={section} title="Beats" actions={header && <span className="tabular text-[12px] text-muted">{header}</span>}>
      {body}
    </Section>
  );
}

import { api, errorText, plainError, type SampleRun } from "./api";
import { useAgent, type ChatItem, type Step } from "./agent";
import { currentEpoch, isCaptionTrack, projectDuration, stopAiRun, useEditor } from "./store";
import type { Project } from "./types";

/**
 * The AI turn of the tour, on the sample video: no agent runs. Nuzky itself makes the edit as one real run
 * (`sample_run`), so the project changes live, the editor locks, Stop works and the run card's Undo takes it all back.
 */
export const SAMPLE_REQUEST = "Cut the retakes and long pauses, then add captions.";

const READ = { id: "demo-read", title: "Read the transcript" };
const CUT = { id: "demo-cut", title: "Cutting retakes and pauses" };
const CAPTIONS = { id: "demo-captions", title: "Adding captions" };

/** How long reading the transcript seems to take: long enough to see the step, short enough not to wait for it. */
const READING_MS = 700;

type Turn = { epoch: string | undefined; cancelled: boolean };
let turn: Turn | null = null;
let outcome: "done" | "stopped" | "failed" | null = null;

/** How the last turn ended; set before the panel goes idle, so whoever watches it going idle can read it. */
export const lastDemoOutcome = () => outcome;

const count = (n: number, word: string) => (n === 0 ? "" : `${n} ${word}${n === 1 ? "" : "s"}`);

/** The reply, from what the edit really did. */
export function demoReply(r: SampleRun): string {
  const cut = [count(r.words, "repeated word"), count(r.pauses, "long pause")].filter(Boolean);
  const captions = r.captions > 0 ? `added ${count(r.captions, "caption")}` : "";
  if (!cut.length) return captions ? `Done. I found nothing to cut and ${captions}.` : "Done. I found nothing to cut.";
  const shorter = ((r.beforeUs - r.afterUs) / 1_000_000).toFixed(1);
  return `Done. I cut ${cut.join(" and ")}, so the video is ${shorter} s shorter${captions ? `, and ${captions}` : ""}.`;
}

/** Why the request cannot be sent now, or null. */
export function demoBlocked(): string | null {
  if (useAgent.getState().status !== "idle") return "Wait for the answer, or stop it";
  if (useEditor.getState().aiRun) return "Another agent is editing. Wait for it, or stop it in the top bar";
  return null;
}

const captionCount = (p: Project | undefined) => p?.tracks.filter(isCaptionTrack).reduce((n, t) => n + t.clips.length, 0) ?? 0;

/** Sets a step of the turn, adding it after the others when it is new. */
function setStep({ id, title }: { id: string; title: string }, status: Step["status"]) {
  const items = [...useAgent.getState().items];
  const last = items[items.length - 1];
  const steps = last?.kind === "steps" ? last.steps : [];
  const step: Step = { id, title, status, detail: null };
  const next = steps.some((s) => s.id === id) ? steps.map((s) => (s.id === id ? step : s)) : [...steps, step];
  if (last?.kind === "steps") items[items.length - 1] = { kind: "steps", steps: next };
  else items.push({ kind: "steps", steps: next });
  useAgent.setState({ items });
}

const stepStatus = (id: string) => {
  const last = useAgent.getState().items.findLast((i) => i.kind === "steps");
  return last?.kind === "steps" ? last.steps.find((s) => s.id === id)?.status : undefined;
};

/** Resolves once `done` holds, or after `ms`. */
function until(done: () => boolean, ms: number) {
  return new Promise<void>((resolve) => {
    if (done()) return resolve();
    const timer = window.setTimeout(finish, ms);
    const off = useEditor.subscribe(() => done() && finish());
    function finish() {
      window.clearTimeout(timer);
      off();
      resolve();
    }
  });
}

/** Sends the request in the field and plays the turn: the steps appear as the edit happens, then the reply and the run's card. */
export async function sendDemo() {
  const s = useAgent.getState();
  const text = s.draft.trim();
  if (!text || demoBlocked()) return;
  const mine: Turn = { epoch: currentEpoch(), cancelled: false };
  turn = mine;
  useAgent.setState({
    items: [...s.items.filter((i) => i.kind !== "error"), { kind: "user", text, context: null, at: Date.now() }],
    draft: "",
    status: "working",
    startedAt: Date.now(),
    demoItems: true,
  });
  // Another project opened meanwhile: the conversation is that project's now.
  const live = () => turn === mine && currentEpoch() === mine.epoch;
  setStep(READ, "running");
  await new Promise((resolve) => window.setTimeout(resolve, READING_MS));
  if (!live()) return;
  setStep(READ, "done");
  if (mine.cancelled) return finish(mine, null);
  setStep(CUT, "running");
  const start = useEditor.getState().snap?.project;
  const length = start ? projectDuration(start) : 0;
  const captions = captionCount(start);
  // The steps follow the project: the cut shortens it, then the captions arrive.
  const follow = useEditor.subscribe((st, prev) => {
    if (!live() || st.snap === prev.snap || !st.snap) return;
    if (stepStatus(CUT.id) === "running" && projectDuration(st.snap.project) < length) {
      setStep(CUT, "done");
      setStep(CAPTIONS, "running");
    }
    if (stepStatus(CAPTIONS.id) === "running" && captionCount(st.snap.project) > captions) setStep(CAPTIONS, "done");
  });
  // A Stop pressed while the run was still opening stops it as soon as it is open.
  const stopOnOpen = useEditor.subscribe((st, prev) => {
    if (st.aiRun && !prev.aiRun && mine.cancelled && live()) void stopAiRun();
  });
  try {
    const result = await api.sampleRun(mine.epoch);
    // The run's card comes with its end; it is drawn from what the run changed.
    await until(() => !useEditor.getState().aiRun, 3000);
    if (live()) finish(mine, result);
  } catch (e) {
    const error = errorText(e);
    if (!live() || error.startsWith("EPOCH_CHANGED")) return;
    await until(() => !useEditor.getState().aiRun, 3000);
    if (mine.cancelled || error.startsWith("CANCELLED")) finish(mine, null);
    else fail(mine, text, plainError(error));
  } finally {
    follow();
    stopOnOpen();
  }
}

/** Ends the turn: stopped with `result` null, else with the reply. The run's card comes last, as after an agent's turn. */
function finish(mine: Turn, result: SampleRun | null) {
  if (turn === mine) turn = null;
  const { items } = useAgent.getState();
  const from = items.map((i) => i.kind).lastIndexOf("user") + 1;
  const stopped = result === null;
  const turnItems = items.slice(from).map((it): ChatItem => (it.kind === "steps" ? { ...it, steps: it.steps.map((x) => (x.status === "running" ? { ...x, status: stopped ? "stopped" : "done" } : x)) } : it));
  const runs = turnItems.filter((it) => it.kind === "run").map((it) => (stopped ? { ...it, stopped: true } : it));
  const reply: ChatItem[] = result ? [{ kind: "agent", text: demoReply(result) }] : [];
  const end: ChatItem[] = stopped && !runs.length ? [{ kind: "stopped" }] : runs;
  outcome = stopped ? "stopped" : "done";
  useAgent.setState({ items: [...items.slice(0, from), ...turnItems.filter((it) => it.kind !== "run"), ...reply, ...end], status: "idle" });
}

/** The edit could not run: the request goes back into the field, and a toast says why. */
function fail(mine: Turn, text: string, why: string) {
  if (turn === mine) turn = null;
  const { items } = useAgent.getState();
  const from = items.map((i) => i.kind).lastIndexOf("user");
  // What it changed before failing stays with its card; otherwise nothing of the turn stays.
  const kept = items.slice(from).some((it) => it.kind === "run");
  // With its card the tour goes on to it; without, it asks again.
  outcome = kept ? "done" : "failed";
  useAgent.setState({
    items: kept ? items.map((it) => (it.kind === "steps" ? { ...it, steps: it.steps.map((x) => (x.status === "running" ? { ...x, status: "failed" } : x)) } : it)) : items.slice(0, from),
    draft: kept ? "" : text,
    status: "idle",
  });
  useEditor.getState().toast({ kind: "error", text: `The sample edit didn't run. ${why}` });
}

/** Stop in the panel. Stop and edit in the top bar ends the run itself, and the turn ends stopped the same way. */
export function stopDemo() {
  const mine = turn;
  if (!mine || useAgent.getState().status !== "working") return;
  mine.cancelled = true;
  useAgent.setState({ status: "stopping" });
  if (useEditor.getState().aiRun) void stopAiRun();
}

import { AlertTriangle, RotateCcw, Unlink, X } from "lucide-react";
import { LIMITS } from "../../lib/limits";
import { loadLuts, lutOf, profileOf, useLuts } from "../../lib/color";
import { ADJUST_ROWS, NO_ADJUST, SPEED_PRESETS, sameAdjust } from "../../lib/presets";
import { useEffect, useState } from "react";
import { api, plainError } from "../../lib/api";
import { detachAudio, detachBlocker, editClip, editClips, findClip, maxFadeUs, setVoice, useEditor, useJobs, type VoiceChange } from "../../lib/store";
import { US, formatDuration } from "../../lib/time";
import type { Adjust, Asset, Clip, Mono } from "../../lib/types";
import { Button, Checkbox, IconButton, ProgressBar, Section, Segmented, Slider } from "../ui";
import { QUIET } from "./Header";

type Media = Extract<Clip["content"], { type: "media" }>;

/** The log profile and LUT of the clip, chosen in Filters: Strength for the LUT and × to take either off. */
export function ColorSection({ clip, content }: { clip: Clip; content: Media }) {
  const edit = useEditor((s) => s.edit);
  const luts = useLuts((s) => s.list);
  useEffect(() => {
    if (luts === null) void loadLuts();
  }, [luts]);
  const profile = profileOf(content.log ?? "none");
  const lut = lutOf(content.lut);
  if (!profile && !lut) return null;
  const missing = lut && luts !== null && !luts.some((l) => l.id === lut.id);
  const row = (name: string, remove: string, onRemove: () => void) => (
    <div className="flex min-h-6 items-center justify-between gap-2">
      <span className="truncate text-[12px] font-medium text-fg/80">{name}</span>
      <IconButton label={remove} className={QUIET} onClick={onRemove}>
        <X size={14} />
      </IconButton>
    </div>
  );
  return (
    <Section title="Colour">
      {profile && (
        <div className="flex flex-col gap-0.5" data-log={profile.id}>
          {row(profile.name, `Show the clip as recorded, without ${profile.name}`, () => edit({ type: "updateClip", clipId: clip.id, log: "none" }))}
          {profile.recordedOn && <span className="text-[12px] text-muted">{profile.recordedOn}</span>}
        </div>
      )}
      {lut && (
        <div className={`flex flex-col gap-2 ${profile ? "border-t border-white/[.07] pt-3" : ""}`} data-lut={lut.name}>
          {row(lut.name, `Remove ${lut.name}`, () => edit({ type: "updateClip", clipId: clip.id, lut: { type: "none" } }))}
          {missing ? (
            <p role="status" className="flex items-start gap-1.5 text-[12px] text-muted">
              <AlertTriangle size={14} className="mt-px shrink-0 text-warn" />
              <span>{`This LUT isn't on this computer, so the clip plays and exports without it. Import ${lut.name}.cube in Filters to use it.`}</span>
            </p>
          ) : (
            <Slider
              label="Strength"
              name={`${lut.name} strength`}
              value={lut.strength * 100}
              min={0}
              max={100}
              step={1}
              unit="%"
              format={(v) => String(Math.round(v))}
              onChange={(v) =>
                editClip(
                  clip.id,
                  (c) => {
                    const had = c.content.type === "media" ? lutOf(c.content.lut) : null;
                    return had ? { type: "updateClip", clipId: c.id, lut: { ...had, strength: v / 100 } } : null;
                  },
                  `${clip.id}:lut:strength`,
                )
              }
            />
          )}
        </div>
      )}
    </Section>
  );
}

export function AdjustSection({ clip, content }: { clip: Clip; content: Media }) {
  const edit = useEditor((s) => s.edit);
  const adjust = content.adjust;
  // Built from the latest confirmed values, so a second slider never overwrites the first.
  const set = (key: keyof Adjust, v: number) =>
    editClip(clip.id, (c) => (c.content.type === "media" ? { type: "updateClip", clipId: c.id, adjust: { ...c.content.adjust, [key]: v / 100 } } : null), `${clip.id}:adjust:${key}`);
  return (
    <Section
      title="Adjust"
      actions={
        <IconButton label="Reset adjustments" className={QUIET} disabled={sameAdjust(adjust, NO_ADJUST)} onClick={() => edit({ type: "updateClip", clipId: clip.id, adjust: NO_ADJUST })}>
          <RotateCcw size={14} />
        </IconButton>
      }
    >
      {ADJUST_ROWS.map((r) => (
        <Slider key={r.key} label={r.label} value={adjust[r.key] * 100} min={r.min} max={100} step={1} format={(v) => String(Math.round(v))} onChange={(v) => set(r.key, v)} />
      ))}
    </Section>
  );
}

export function SpeedSection({ clip, content, asset }: { clip: Clip; content: Media; asset?: Asset }) {
  const edit = useEditor((s) => s.edit);
  const speed = content.speed;
  const set = (v: number, key = "speed") => edit({ type: "updateClip", clipId: clip.id, speed: Math.round(v * 100) / 100 }, `${clip.id}:${key}`);
  const sourceUs = clip.durationUs * speed;
  return (
    <Section title="Speed">
      <Segmented
        label="Speed presets"
        value={SPEED_PRESETS.find((p) => Math.abs(p - speed) < 0.005) ?? null}
        onChange={(p) => set(p, `speed-preset:${Date.now()}`)}
        options={SPEED_PRESETS.map((p) => ({ id: p, label: `${p}x` }))}
      />
      {/* Logarithmic so 0.1x–1x gets as much travel as 1x–10x. */}
      <Slider
        label="Speed"
        value={Math.log10(speed)}
        min={Math.log10(LIMITS.minSpeed)}
        max={Math.log10(LIMITS.maxSpeed)}
        step={0.01}
        unit="x"
        format={(v) => (10 ** v).toFixed(2)}
        parse={(s) => Math.log10(Number(s.replace(",", ".").replace(/[^\d.]/g, "")))}
        onChange={(v) => set(10 ** v)}
      />
      <dl className="tabular grid grid-cols-[78px_1fr] gap-x-2 gap-y-1.5 text-[12px]">
        <dt className="text-muted">Duration</dt>
        <dd className="text-fg">{formatDuration(clip.durationUs)}</dd>
        <dt className="text-muted">Source used</dt>
        <dd className="text-fg">{formatDuration(sourceUs)}</dd>
      </dl>
      {asset?.hasAudio && speed !== 1 && (
        <Checkbox
          label="Keep pitch"
          checked={content.keepPitch ?? false}
          title="The voice sounds as high as at 1x. Off, it gets higher when faster and lower when slower."
          onChange={(v) => edit({ type: "updateClip", clipId: clip.id, keepPitch: v })}
        />
      )}
    </Section>
  );
}

/** What Reduce echo and Even out volume start at when turned on. */
const DEREVERB = 0.8;
const LEVELER = 0.5;

const signedDb = (db: number) => `${db < -0.05 ? "−" : "+"}${Math.abs(db).toFixed(1)} dB`;

/** The gain Match loudness gives this clip in dB once its file's loudness is measured, else null. */
function useMatchGain(clip: Clip | undefined) {
  const c = clip?.content.type === "media" && clip.content.normalize ? clip.content : undefined;
  const id = c ? clip!.id : null;
  // It follows the sound the other Voice settings make, and is asked again when a preparation of the file ends.
  const key = c ? `${id}:${c.assetId}:${c.cleanVoice}:${c.dereverb ?? 0}:${c.leveler ?? 0}` : null;
  const jobs = useEditor((s) => (c ? ["voice", "dereverb", "level"].map((k) => s.jobs[`${k}:${c.assetId}`]?.status).join() : ""));
  const [result, setResult] = useState<{ key: string; db: number | null }>();
  useEffect(() => {
    let live = true;
    // Once a slider stops moving, not on every step of a drag.
    const timer = id && key ? setTimeout(() => api.matchGain(id).then((db) => live && setResult({ key, db }), () => live && setResult({ key, db: null })), 150) : undefined;
    return () => {
      live = false;
      clearTimeout(timer);
    };
  }, [id, key, jobs]);
  return result && result.key === key ? result.db : null;
}

/**
 * The preparation of a Voice setting's sound for these files, under the setting: how far it is, or why it failed
 * and what that means.
 */
function JobRow({ kind, assetIds, doing, failed, meanwhile }: { kind: string; assetIds: string[]; doing: string; failed: string; meanwhile: string }) {
  const job = useJobs(kind, assetIds);
  if (job?.failed != null) {
    return (
      <p role="status" className="flex items-start gap-1.5 text-[12px] text-muted">
        <AlertTriangle size={14} className="mt-px shrink-0 text-danger" />
        {`${failed}: ${plainError(job.failed).split(": ").pop()}. ${meanwhile}`}
      </p>
    );
  }
  if (job?.progress == null) return null;
  return (
    <div className="flex flex-col gap-1.5" role="status">
      <span className="tabular text-[12px] text-muted">
        {doing}
        {job.progress > 0 && ` · ${Math.round(job.progress * 100)}%`}
      </span>
      <ProgressBar value={job.progress} label={doing} />
    </div>
  );
}

/**
 * A setting with a strength for one or more clips: checking turns it on at `start` and keeps the strength of
 * clips that have it; the strength shows once all of them have it on.
 */
function StrengthRow({ clips, field, label, by, title, start, coalesce }: { clips: Clip[]; field: "dereverb" | "leveler"; label: string; by: string; title: string; start: number; coalesce: string }) {
  const values = clips.map((c) => (c.content.type === "media" ? (c.content[field] ?? 0) : 0));
  const on = values.filter((v) => v > 0);
  const all = on.length === values.length;
  const ids = clips.map((c) => c.id);
  const change = (v: number): VoiceChange => (field === "dereverb" ? { dereverb: v } : { leveler: v });
  const turn = (v: boolean) =>
    editClips(ids, (c) => (c.content.type !== "media" || (v && (c.content[field] ?? 0) > 0) ? null : { type: "updateClip", clipId: c.id, ...change(v ? start : 0) }));
  return (
    <>
      <Checkbox label={label} checked={all} mixed={!all && on.length > 0} title={title} onChange={turn} />
      {all && (
        <Slider
          label={by}
          mixed={on.some((v) => v !== on[0])}
          value={on[0] * 100}
          min={1}
          max={100}
          step={1}
          unit="%"
          format={(v) => String(Math.round(v))}
          onChange={(v) => setVoice(ids, change(v / 100), `${coalesce}:${field}`)}
        />
      )}
    </>
  );
}

/**
 * The Voice section of one or more clips with sound, in the order the sound goes through it. Each file's sound for
 * a setting is prepared once in the background; meanwhile playback keeps the sound it has and a row under the
 * setting shows how far the work is.
 */
export function VoiceSection({ clips, coalesce }: { clips: Clip[]; coalesce: string }) {
  const ids = clips.map((c) => c.id);
  const media = clips.flatMap((c) => (c.content.type === "media" ? [c.content] : []));
  const clean = media.map((m) => m.cleanVoice);
  const normalize = media.map((m) => !!m.normalize);
  const files = (on: (m: (typeof media)[number]) => boolean) => [...new Set(media.filter(on).map((m) => m.assetId))];
  const single = clips.length === 1 ? clips[0] : undefined;
  const matched = useMatchGain(single);
  const volume = single?.content.type === "media" ? single.content.volume : 0;
  const gain = matched !== null && volume > 0 ? matched + 20 * Math.log10(volume) : null;
  return (
    <Section title="Voice">
      <Checkbox
        label="Clean voice"
        checked={clean.every(Boolean)}
        mixed={!clean.every(Boolean) && clean.some(Boolean)}
        title="Less background noise, rumble and harsh s sounds in speech"
        onChange={(v) => setVoice(ids, { cleanVoice: v })}
      />
      <JobRow kind="voice" assetIds={files((m) => m.cleanVoice)} doing="Cleaning voice" failed="The voice could not be cleaned" meanwhile="It plays as recorded; turn Clean voice off to export." />
      <StrengthRow clips={clips} field="dereverb" label="Reduce echo" by="Reduce by" title="Takes the ring of the room out of speech" start={DEREVERB} coalesce={coalesce} />
      <JobRow kind="dereverb" assetIds={files((m) => (m.dereverb ?? 0) > 0)} doing="Reducing echo" failed="The echo could not be taken out" meanwhile="It plays as recorded; turn Reduce echo off to export." />
      <StrengthRow clips={clips} field="leveler" label="Even out volume" by="Even out by" title="Brings quiet and loud passages of speech closer together" start={LEVELER} coalesce={coalesce} />
      <Checkbox
        label="Match loudness"
        checked={normalize.every(Boolean)}
        mixed={!normalize.every(Boolean) && normalize.some(Boolean)}
        title="Plays every recording equally loud, at −16 LUFS; Volume still applies on top"
        onChange={(v) => setVoice(ids, { normalize: v })}
        detail={gain !== null && <span title={`Match loudness ${signedDb(matched!)} with the volume ${signedDb(20 * Math.log10(volume))}`}>{signedDb(gain)}</span>}
      />
      <JobRow
        kind="level"
        assetIds={files((m) => !!m.normalize || (m.leveler ?? 0) > 0)}
        doing="Measuring loudness"
        failed="The loudness could not be measured"
        meanwhile="It plays at its own level; turn Even out volume and Match loudness off to export."
      />
    </Section>
  );
}

/** How far a clip goes down under speech when Lower under speech is turned on, in dB. */
const DUCK_DB = 12;

/**
 * Lower under speech (ducking) for one or more clips with sound: they go down while a video's own
 * sound has speech and come back in the pauses. The strength shows once all of them have it on.
 */
export function DuckingRow({ clips, coalesce }: { clips: Clip[]; coalesce: string }) {
  const levels = clips.map((c) => (c.content.type === "media" ? (c.content.duckDb ?? 0) : 0));
  const on = levels.filter((v) => v > 0);
  const all = on.length === levels.length;
  const ids = clips.map((c) => c.id);
  // Checking keeps the strength of clips that already have it.
  const turn = (v: boolean) =>
    editClips(ids, (c) => (c.content.type !== "media" || (v && (c.content.duckDb ?? 0) > 0) ? null : { type: "updateClip", clipId: c.id, duckDb: v ? DUCK_DB : 0 }));
  return (
    <>
      <Checkbox
        label="Lower under speech"
        tour="duck"
        checked={all}
        mixed={!all && on.length > 0}
        title="Turns this sound down while someone speaks in a video and back up in the pauses"
        onChange={turn}
      />
      {all && (
        <Slider
          label="Lower by"
          mixed={on.some((v) => v !== on[0])}
          value={on[0]}
          min={1}
          max={LIMITS.maxDuckDb}
          step={1}
          unit="dB"
          format={(v) => String(Math.round(v))}
          onChange={(v) => editClips(ids, (c) => ({ type: "updateClip", clipId: c.id, duckDb: v }), `${coalesce}:duck`)}
        />
      )}
    </>
  );
}

/** The side of the file's sound that has it all when a microphone recorded on one side only; asked again once its sound is prepared. */
function useSoundOnlyIn(assetId: string | undefined) {
  const preparing = useEditor((s) => (assetId ? s.jobs[`audio:${assetId}`]?.status : undefined));
  const [result, setResult] = useState<{ assetId: string; side: Mono | null }>();
  useEffect(() => {
    let live = true;
    if (assetId) api.soundOnlyIn(assetId).then((side) => live && setResult({ assetId, side }), () => live && setResult({ assetId, side: null }));
    return () => {
      live = false;
    };
  }, [assetId, preparing]);
  return result && result.assetId === assetId ? result.side : null;
}

const MONO_FROM: { id: Exclude<Mono, "off">; label: string; title: string }[] = [
  { id: "both", label: "Both", title: "Both sides mixed into one" },
  { id: "left", label: "Left", title: "Only the left side, in both ears" },
  { id: "right", label: "Right", title: "Only the right side, in both ears" },
];

/**
 * Mono for one or more clips with sound: one sound in both ears, from both sides of the file or from one. With one
 * clip whose file has sound on one side only, the row says which side, and checking it plays that side in both ears.
 */
export function MonoRow({ clips }: { clips: Clip[] }) {
  const sides = clips.map((c) => (c.content.type === "media" ? (c.content.mono ?? "off") : "off"));
  const on = sides.filter((m) => m !== "off");
  const all = on.length === sides.length;
  const ids = clips.map((c) => c.id);
  const single = clips.length === 1 && clips[0].content.type === "media" ? clips[0].content : undefined;
  const onlyIn = useSoundOnlyIn(single?.assetId);
  const oneEar = !!onlyIn && (single?.mono ?? "off") === "off";
  const set = (mono: Mono) => editClips(ids, (c) => ({ type: "updateClip", clipId: c.id, mono }));
  // Checking keeps the side of clips that already have one, and starts a file recorded on one side on that side.
  const turn = (v: boolean) =>
    editClips(ids, (c) => (c.content.type !== "media" || (v && (c.content.mono ?? "off") !== "off") ? null : { type: "updateClip", clipId: c.id, mono: v ? (onlyIn ?? "both") : "off" }));
  return (
    <>
      <Checkbox
        label="Mono"
        checked={all}
        mixed={!all && on.length > 0}
        title={oneEar ? `Only the ${onlyIn} side has sound, so it plays in one ear. Mono plays it in both.` : "Plays one sound in both ears"}
        onChange={turn}
        detail={
          oneEar && (
            <span role="status" className="flex items-center gap-1">
              <AlertTriangle size={12} className="shrink-0 text-warn" />
              {onlyIn === "left" ? "Left side only" : "Right side only"}
            </span>
          )
        }
      />
      {all && (
        <div className="flex flex-col gap-1.5">
          <span className="text-[12px] text-muted">From</span>
          <Segmented label="Mono from" options={MONO_FROM} value={on.every((m) => m === on[0]) ? (on[0] as Exclude<Mono, "off">) : null} onChange={set} />
        </div>
      )}
    </>
  );
}

export function AudioSection({ clip, content }: { clip: Clip; content: Media }) {
  const project = useEditor((s) => s.snap!.project);
  const edit = useEditor((s) => s.edit);
  const maxFade = maxFadeUs(clip.durationUs) / US;
  const blocker = detachBlocker(project, clip);
  // Sound already detached to an audio track has nothing left to detach.
  const isVideo = project.assets.find((a) => a.id === content.assetId)?.kind === "video" && findClip(project, clip.id)?.track.kind !== "audio";
  return (
    <Section title="Audio">
      <Slider
        label="Volume"
        value={content.volume * 100}
        min={0}
        max={200}
        step={1}
        unit="%"
        format={(v) => String(Math.round(v))}
        onChange={(v) => edit({ type: "updateClip", clipId: clip.id, volume: v / 100 }, `${clip.id}:volume`)}
      />
      <Slider
        label="Fade in"
        value={content.fadeInUs / US}
        min={0}
        max={maxFade}
        step={0.1}
        unit="s"
        format={(v) => v.toFixed(1)}
        onChange={(v) => edit({ type: "updateClip", clipId: clip.id, fadeInUs: Math.round(v * US) }, `${clip.id}:fadeIn`)}
      />
      <Slider
        label="Fade out"
        value={content.fadeOutUs / US}
        min={0}
        max={maxFade}
        step={0.1}
        unit="s"
        format={(v) => v.toFixed(1)}
        onChange={(v) => edit({ type: "updateClip", clipId: clip.id, fadeOutUs: Math.round(v * US) }, `${clip.id}:fadeOut`)}
      />
      <DuckingRow clips={[clip]} coalesce={clip.id} />
      <MonoRow clips={[clip]} />
      {isVideo && (
        <Button className="w-full" disabled={!!blocker} disabledReason={blocker ?? undefined} title="Move the sound to its own audio track" onClick={() => detachAudio([clip.id])}>
          <Unlink size={14} /> Detach audio
        </Button>
      )}
    </Section>
  );
}

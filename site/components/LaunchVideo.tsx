"use client";

import { type PointerEvent, useEffect, useRef, useState } from "react";
import { Pause, Play, Volume2, VolumeX } from "lucide-react";
import { useView } from "@/lib/playing";

// The same minute in two cuts: the wide one for a computer, and on a phone the tall one, whose text and app stay
// large enough to read. The files live on files.tmach.dev because the repository refuses video. AV1 is half the
// size of H.264 at the same quality, but decoded without a chip for it, it drains a laptop's or a phone's battery
// and can stutter, so it plays only where the hardware decodes it or where the browser cannot play H.264 at all.
const cuts = {
  wide: {
    av1: "https://files.tmach.dev/nuzky-launch-wide-av1-aa93149c9d2f45aeac44.mp4",
    h264: "https://files.tmach.dev/nuzky-launch-wide-h264-e5996f08f93d46129516.mp4",
    poster: "/media/launch-wide.webp",
    width: 1920,
    height: 1080,
    bitrate: 716_000,
  },
  tall: {
    av1: "https://files.tmach.dev/nuzky-launch-tall-av1-30c4336fe4104e09964a.mp4",
    h264: "https://files.tmach.dev/nuzky-launch-tall-h264-22ef72a073894885890e.mp4",
    poster: "/media/launch-tall.webp",
    width: 1080,
    height: 1920,
    bitrate: 801_000,
  },
};

const AV1 = 'video/mp4; codecs="av01.0.09M.08"';
const H264 = 'video/mp4; codecs="avc1.64002A"';

async function preferAv1(cut: (typeof cuts)["wide"]) {
  if (!document.createElement("video").canPlayType(H264)) return true;
  // After a seek between two key frames of these AV1 files, WebKit shows a black or broken picture until the next key
  // frame four seconds on, so Safari and every browser on an iPhone get H.264.
  if (navigator.vendor.startsWith("Apple")) return false;
  const video = { contentType: AV1, width: cut.width, height: cut.height, bitrate: cut.bitrate, framerate: 60 };
  const info = await navigator.mediaCapabilities?.decodingInfo({ type: "file", video }).catch(() => undefined);
  return !!info?.supported && info.powerEfficient;
}

const clock = (s: number) => `${Math.floor(s / 60)}:${String(Math.floor(s % 60)).padStart(2, "0")}`;

// Not a range input: iPhone Safari moves one only by its thumb, so a tap on the line would not seek.
function Slider({
  label,
  value,
  max,
  step,
  text,
  onChange,
  glide = false,
  className = "",
}: {
  label: string;
  value: number;
  max: number;
  step: number;
  text: (value: number) => string;
  onChange: (value: number) => void;
  glide?: boolean;
  className?: string;
}) {
  const [drag, setDrag] = useState<number>();
  const shown = drag ?? value;
  // The knob's centre stays 6 px inside each end, so the knob never pokes past the line.
  const pos = `calc(${max > 0 ? Math.min(shown, max) / max : 0} * (100% - 12px) + 6px)`;
  const set = (v: number) => onChange(Math.min(max, Math.max(0, v)));
  const follow = (e: PointerEvent<HTMLDivElement>) => {
    const r = e.currentTarget.getBoundingClientRect();
    const v = (Math.min(Math.max(e.clientX - r.left, 0), r.width) / r.width) * max;
    setDrag(v);
    set(v);
  };
  const keys: Record<string, number> = { ArrowRight: value + step, ArrowUp: value + step, ArrowLeft: value - step, ArrowDown: value - step, Home: 0, End: max };
  // Between two reports of the time the line glides, so a playing video moves it smoothly.
  const ease = drag === undefined && glide ? "duration-[250ms] ease-linear" : "duration-0";

  return (
    <div
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={shown}
      aria-valuetext={text(shown)}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        follow(e);
      }}
      onPointerMove={(e) => drag !== undefined && follow(e)}
      onLostPointerCapture={() => setDrag(undefined)}
      onKeyDown={(e) => {
        if (!(e.key in keys)) return;
        e.preventDefault();
        set(keys[e.key]);
      }}
      className={`group/slider relative flex shrink-0 cursor-pointer touch-none items-center rounded-full after:absolute after:inset-x-0 after:-inset-y-2 ${className}`}
    >
      <span className="h-[3px] w-full rounded-full bg-white/30" />
      <span className={`absolute left-0 h-[3px] rounded-full bg-fg transition-[width] ${ease}`} style={{ width: pos }} />
      <span
        className={`absolute size-3 -translate-x-1/2 rounded-full bg-fg transition-[left,scale] group-hover/slider:scale-100 group-focus-visible/slider:scale-100 pointer-coarse:scale-100 ${drag === undefined ? "scale-0" : ""} ${ease}`}
        style={{ left: pos }}
      />
    </div>
  );
}

// The bar itself lets clicks through to the picture; only its controls take them, and only while they show.
const live = "group-data-[shown=true]/bar:pointer-events-auto group-has-[:focus-visible]/bar:pointer-events-auto";
const icon = `relative flex size-10 shrink-0 items-center justify-center rounded-full text-fg/90 transition-colors after:absolute after:-inset-1 hover:bg-white/10 hover:text-fg ${live}`;

// Plays muted while at least a tenth of it is on screen, and the visitor turns the sound on with a click. A tenth
// because on a 1366 × 768 laptop the first screen shows only a fifth of it under the headline.
// Nothing downloads until the page has loaded and the video comes on screen. With reduced motion, or where the
// browser refuses to start a video by itself (Low Power Mode on an iPhone), it waits on a still from the video
// until the visitor plays it, and the still stays when the file cannot load at all. Once the visitor plays or
// pauses it, it stays that way, also after scrolling away and back.
export function LaunchVideo({ className = "" }: { className?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const at = useRef(0);
  const touch = useRef(false);
  const sleep = useRef<ReturnType<typeof setTimeout>>(undefined);
  const controls = useRef<HTMLDivElement>(null);
  const [file, setFile] = useState<{ src: string; poster: string; at: number }>();
  const [sound, setSound] = useState(false);
  const [choice, setChoice] = useState<"play" | "pause">();
  const [refused, setRefused] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ready, setReady] = useState(false);
  const [paused, setPaused] = useState(true);
  const [awake, setAwake] = useState(false);
  const [time, setTime] = useState(0);
  const [glide, setGlide] = useState(false);
  const [length, setLength] = useState(0);
  const [volume, setVolume] = useState(1);
  const [volumeWorks, setVolumeWorks] = useState(false);
  const { shown, still } = useView(box, 0.1);
  const waiting = !choice && (still || refused);
  const playing = shown && (choice ? choice === "play" : !waiting);
  const bar = paused || awake;

  // The breakpoint is the one that switches the box between 9:16 and 16:9; both cuts run on the same timeline, so
  // turning a phone carries on where it was, even when it turns back before the other cut has loaded. The same
  // element only changes its file, because Safari lets the sound play only in the element the visitor tapped. The
  // video waits for the page's load event, because a loading video holds that event back and the page's own
  // pictures should come first.
  useEffect(() => {
    const computer = window.matchMedia("(min-width: 40rem)");
    const loaded = new Promise((done) => (document.readyState === "complete" ? done(0) : window.addEventListener("load", done, { once: true })));
    const pick = async () => {
      const cut = computer.matches ? cuts.wide : cuts.tall;
      const av1 = await preferAv1(cut);
      await loaded;
      if (cut !== (computer.matches ? cuts.wide : cuts.tall)) return;
      if (video.current?.readyState) at.current = video.current.currentTime;
      setFile({ src: av1 ? cut.av1 : cut.h264, poster: cut.poster, at: at.current });
      setFailed(false);
    };
    pick();
    computer.addEventListener("change", pick);
    return () => computer.removeEventListener("change", pick);
  }, []);

  useEffect(() => {
    const v = video.current;
    if (!v || !file) return;
    if (!playing) {
      v.pause();
      return;
    }
    v.muted = !sound;
    v.play().catch((e: DOMException) => {
      if (e.name !== "NotAllowedError") return;
      if (v.muted) setRefused(true);
      else setSound(false);
    });
  }, [playing, sound, file]);

  // iPhone and iPad Safari ignore `volume` and leave the level to the device's buttons, as every phone does, so the
  // slider shows only where setting the level works and a mouse can reach it.
  useEffect(() => {
    const probe = document.createElement("video");
    probe.volume = 0.5;
    setVolumeWorks(probe.volume === 0.5 && window.matchMedia("(pointer: fine)").matches);
  }, []);

  useEffect(() => () => clearTimeout(sleep.current), []);

  // Like a video player's bar: it stays while the video is paused or the pointer rests on a control, and otherwise
  // shows for a moment after the pointer moves over the video or a tap.
  const wake = () => {
    setAwake(true);
    clearTimeout(sleep.current);
    sleep.current = setTimeout(() => (controls.current?.matches(":hover") ? wake() : setAwake(false)), 2500);
  };

  // Sound turned on at volume 0 would stay silent, so it comes back at full volume.
  const audible = () => {
    const v = video.current;
    if (!v || v.volume > 0) return;
    v.volume = 1;
    setVolume(1);
  };
  // A phone plays a video with sound only when play() runs inside the visitor's own tap, not in a later effect.
  const play = (withSound: boolean) => {
    const v = video.current;
    if (!v) return;
    if (withSound) audible();
    v.muted = !withSound;
    v.play().catch(() => {});
    setSound(withSound);
    setChoice("play");
  };
  const pause = () => {
    video.current?.pause();
    setChoice("pause");
  };
  const toggleSound = () => {
    if (!paused) return play(!sound);
    if (!sound) audible();
    if (video.current) video.current.muted = sound;
    setSound(!sound);
  };
  const seek = (t: number) => {
    if (video.current) video.current.currentTime = t;
    setGlide(false);
    setTime(t);
    wake();
  };
  const level = (value: number) => {
    // The far left of a line 64 px wide is a few percent, not silence, so the last bit counts as 0.
    const l = value < 0.05 ? 0 : value;
    if (video.current) video.current.volume = l;
    setVolume(l);
    if (!sound && l > 0) toggleSound();
  };
  // A click on the picture turns the sound on first and then plays and pauses. A tap on a phone while the bar is
  // hidden only brings the bar back, as a finger has no hover to show it.
  const press = () => {
    const hidden = !bar;
    wake();
    if (!sound) play(true);
    else if (!(touch.current && hidden)) (paused ? play(true) : pause());
  };

  return (
    <div
      ref={box}
      onPointerMove={(e) => e.pointerType === "mouse" && wake()}
      onPointerLeave={(e) => e.pointerType === "mouse" && setAwake(false)}
      onKeyDown={(e) => {
        const key = e.key.toLowerCase();
        if (e.target instanceof HTMLButtonElement && (key === " " || key === "enter")) return;
        if (key === " " || key === "k") {
          e.preventDefault();
          if (paused) play(sound);
          else pause();
        } else if (key === "m") toggleSound();
      }}
      className={`@container relative mx-auto aspect-[9/16] w-full max-w-[calc((100svh-6rem)*9/16)] overflow-hidden rounded-[20px] bg-bg ring-1 ring-white/10 sm:aspect-video sm:max-w-[min(1180px,calc((100svh-min(8rem,10svh))*16/9))] ${className}`}
    >
      {file && (
        <video
          ref={video}
          src={file.src}
          poster={still || refused || failed ? file.poster : undefined}
          muted
          loop
          playsInline
          preload="none"
          disablePictureInPicture
          disableRemotePlayback
          onLoadedMetadata={(e) => {
            if (file.at) e.currentTarget.currentTime = file.at;
          }}
          onLoadedData={() => setReady(true)}
          onDurationChange={(e) => setLength(e.currentTarget.duration)}
          onTimeUpdate={(e) => {
            const t = e.currentTarget.currentTime;
            setGlide(t >= time && t - time < 1);
            setTime(t);
          }}
          onPlay={() => setPaused(false)}
          onPause={() => setPaused(true)}
          onError={() => setFailed(true)}
          onPointerDown={(e) => (touch.current = e.pointerType !== "mouse")}
          onClick={press}
          aria-label="Nuzky in one minute: cutting a talking video by its words, captions, a cover and three finished reels"
          className="size-full cursor-pointer object-cover"
        />
      )}
      {/* The tall cut and a wide one at 900 px or more leave the corner under the labelled switch empty for the whole
          minute; a smaller wide one has room only for the icon. */}
      {file && !failed && (waiting || (ready && !sound && !paused)) && (
        <button
          type="button"
          onClick={() => play(true)}
          aria-label={waiting ? "Play with sound" : "Turn on sound"}
          className={`group absolute right-0 top-0 flex cursor-pointer items-start justify-end rounded-[20px] p-3 focus-visible:outline-offset-[-3px] sm:p-4 ${waiting ? "inset-0" : ""}`}
        >
          <span className="flex h-9 items-center gap-2 rounded-full bg-black/60 px-2.5 text-[13px] font-medium ring-1 ring-inset ring-white/15 transition-colors group-hover:bg-black/75 group-hover:ring-white/40">
            {waiting ? <Play className="size-4" /> : <VolumeX className="size-4" />}
            <span className="pr-1 sm:@max-[900px]:hidden">{waiting ? "Play with sound" : "Turn on sound"}</span>
          </span>
        </button>
      )}
      {/* The video draws its own counter along the bottom in its first seconds, so the shade under the bar is dark
          enough to hide it. */}
      {file && !failed && !waiting && ready && (
        <div
          ref={controls}
          data-shown={bar}
          className={`group/bar pointer-events-none absolute inset-x-0 bottom-0 select-none px-2 pb-1 pt-14 transition-opacity duration-200 has-[:focus-visible]:opacity-100 sm:px-3 ${bar ? "" : "opacity-0"}`}
          // Inline, because Tailwind's gradient classes would add 0.3 kB to the CSS that the first paint waits for.
          style={{ background: "linear-gradient(to top, rgb(0 0 0 / 0.85), rgb(0 0 0 / 0.4) 50%, transparent)" }}
        >
          <Slider
            label="Seek"
            value={time}
            max={length}
            step={5}
            text={(t) => `${clock(t)} of ${clock(Math.round(length))}`}
            onChange={seek}
            glide={glide}
            className={`mx-2 h-4 ${live}`}
          />
          <div className="flex items-center">
            <button type="button" onClick={paused ? () => play(sound) : pause} aria-label={paused ? "Play" : "Pause"} className={icon}>
              {paused ? <Play className="size-5 fill-current" /> : <Pause className="size-5 fill-current" />}
            </button>
            <div className="group/volume flex items-center">
              <button type="button" onClick={toggleSound} aria-label={sound ? "Turn off sound" : "Turn on sound"} className={icon}>
                {sound && volume > 0 ? <Volume2 className="size-5" /> : <VolumeX className="size-5" />}
              </button>
              {volumeWorks && (
                <Slider
                  label="Volume"
                  value={sound ? volume : 0}
                  max={1}
                  step={0.1}
                  text={(l) => `${Math.round(l * 100)} %`}
                  onChange={level}
                  className={`h-8 w-0 opacity-0 transition-[width,opacity,margin] group-focus-within/volume:mr-2 group-focus-within/volume:w-16 group-focus-within/volume:opacity-100 group-hover/volume:mr-2 group-hover/volume:w-16 group-hover/volume:opacity-100 ${live}`}
                />
              )}
            </div>
            <span className="ml-1 text-[13px] tabular-nums text-fg/85">
              {clock(time)} / {clock(Math.round(length))}
            </span>
          </div>
        </div>
      )}
    </div>
  );
}

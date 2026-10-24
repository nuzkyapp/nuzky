"use client";

import { useEffect, useRef, useState } from "react";
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
  const video = { contentType: AV1, width: cut.width, height: cut.height, bitrate: cut.bitrate, framerate: 60 };
  const info = await navigator.mediaCapabilities?.decodingInfo({ type: "file", video }).catch(() => undefined);
  return !!info?.supported && info.powerEfficient;
}

// Plays muted while at least a tenth of it is on screen, and the visitor turns the sound on with a click. A tenth
// because on a 1366 × 768 laptop the first screen shows only a fifth of it under the headline.
// Nothing downloads until the page has loaded and the video comes on screen. With reduced motion, or where the
// browser refuses to start a video by itself (Low Power Mode on an iPhone), it waits on a still from the video
// until the visitor plays it, and the still stays when the file cannot load at all.
export function LaunchVideo({ className = "" }: { className?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const video = useRef<HTMLVideoElement>(null);
  const at = useRef(0);
  const [file, setFile] = useState<{ src: string; poster: string; at: number }>();
  const [sound, setSound] = useState(false);
  const [refused, setRefused] = useState(false);
  const [failed, setFailed] = useState(false);
  const [ready, setReady] = useState(false);
  const [paused, setPaused] = useState(true);
  const { shown, still } = useView(box, 0.1);
  const playing = shown && (sound || !still);

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

  // A phone plays a video with sound only when play() runs inside the visitor's own tap, not in a later effect.
  const toggle = () => {
    const v = video.current;
    if (!v) return;
    v.muted = sound;
    if (!sound) v.play().catch(() => {});
    setSound(!sound);
  };

  const waiting = (still || refused) && paused;
  // With reduced motion the video plays only while the sound is on, so turning the sound off stops it.
  const label = sound ? (still ? "Pause" : "Turn off sound") : waiting ? "Play with sound" : "Turn on sound";
  const Icon = sound ? (still ? Pause : Volume2) : waiting ? Play : VolumeX;

  return (
    <div
      ref={box}
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
          onPlay={() => setPaused(false)}
          onPause={() => setPaused(true)}
          onError={() => setFailed(true)}
          aria-label="Nuzky in one minute: cutting a talking video by its words, captions, a cover and three finished reels"
          className="size-full object-cover"
        />
      )}
      {/* The tall cut and a wide one at 900 px or more leave the corner under the labelled switch empty for the whole
          minute; a smaller wide one has room only for the icon. */}
      {file && !failed && (ready || waiting) && (
        <button
          type="button"
          onClick={toggle}
          aria-label={label}
          className="group absolute inset-0 flex cursor-pointer items-start justify-end rounded-[20px] p-3 focus-visible:outline-offset-[-3px] sm:p-4"
        >
          <span className="flex h-9 items-center gap-2 rounded-full bg-black/60 px-2.5 text-[13px] font-medium ring-1 ring-inset ring-white/15 transition-colors group-hover:bg-black/75 group-hover:ring-white/40">
            <Icon className="size-4" />
            {!sound && <span className="pr-1 sm:@max-[900px]:hidden">{label}</span>}
          </span>
        </button>
      )}
    </div>
  );
}

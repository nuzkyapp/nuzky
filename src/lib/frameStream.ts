import { useEffect, useRef, useState } from "react";

const HEADER = 44;
/** A frame due this much later or earlier than it arrives shows clocks that no longer agree, as after the computer slept. */
const MAX_WAIT_MS = 250;

type Frame = { buf: ArrayBuffer; due: number; playing: boolean };

/**
 * Receives rendered frames from an engine and paints them; `onFrame` gets each one's time and the generation of the
 * transport it was drawn in (`Transport.generation`), which a seek, pause or play changes. A paused frame paints on
 * the next display frame. A playing frame comes ahead of its time and paints on the display frame nearest to when it
 * is due, so frames stay on screen equally long however unevenly they took to draw and arrive.
 */
export function useFrameStream(url: string, canvas: React.RefObject<HTMLCanvasElement | null>, onFrame: (t: number, playing: boolean, generation: number) => void) {
  const frame = useRef(onFrame);
  frame.current = onFrame;
  const [connected, setConnected] = useState(false);
  useEffect(() => {
    if (!url) return;
    let ws: WebSocket | null = null;
    let closed = false;
    let queue: Frame[] = [];
    let raf = 0;
    let retry = 0;
    let painted = 0;
    // The engine's clock minus this page's, in ms, from the frame that arrived soonest. Both clocks run alike, so it
    // only grows as a quicker frame shows the difference better.
    let offset = -Infinity;
    let lastTick = 0;
    let refresh = 1000 / 60;
    // The display frame the last playing frame was painted on, and when that frame was due.
    let anchor: { at: number; due: number } | null = null;
    let generation = -1;

    const paint = (buf: ArrayBuffer) => {
      const el = canvas.current;
      if (!el) return;
      const view = new DataView(buf);
      const w = view.getUint32(4, true);
      const h = view.getUint32(8, true);
      const playing = (view.getUint32(12, true) & 1) === 1;
      const t = Number(view.getBigInt64(16, true));
      const drawnIn = view.getUint32(40, true);
      if (el.width !== w || el.height !== h) {
        el.width = w;
        el.height = h;
      }
      // Opaque: frames have no transparency, and an opaque canvas needs no blending with what is behind it.
      el.getContext("2d", { alpha: false })?.putImageData(new ImageData(new Uint8ClampedArray(buf, HEADER, w * h * 4), w, h), 0, 0);
      // The time of the frame on screen. After a seek the previous frame stays a moment; the UI flows wait for this.
      el.dataset.us = String(t);
      // How many frames this canvas has shown, so a frame drawn after a change can be told from one drawn before it.
      el.dataset.n = String(++painted);
      frame.current(t, playing, drawnIn);
    };

    // What is painted now is on screen from the next refresh on. A frame shows by its time, and while playing it
    // keeps the rhythm of the frame before as long as that stays within a quarter of a refresh of its time, so a
    // frame due right at a refresh does not land on alternate sides of it.
    const ready = (f: Frame, now: number) => {
      const late = now + refresh - f.due;
      if (late > refresh * 1.25) return true;
      if (late < -refresh * 0.25) return false;
      return anchor ? now >= anchor.at + f.due - anchor.due - refresh / 2 : late >= 0;
    };

    const tick = (now: number) => {
      raf = 0;
      if (lastTick && now - lastTick < refresh * 1.5) refresh += (now - lastTick - refresh) * 0.05;
      lastTick = now;
      let shown = -1;
      for (let i = 0; i < queue.length && ready(queue[i], now); i++) shown = i;
      if (shown >= 0) {
        const { buf, due, playing } = queue[shown];
        queue = queue.slice(shown + 1);
        anchor = playing ? { at: now, due } : null;
        paint(buf);
      }
      if (queue.length) raf = requestAnimationFrame(tick);
      else lastTick = 0;
    };

    const receive = (buf: ArrayBuffer) => {
      const view = new DataView(buf);
      if (view.getUint32(0, true) !== 0x32465043) return; // "CPF2"
      const arrived = performance.now();
      const sent = Number(view.getBigInt64(24, true)) / 1000;
      const dueThere = Number(view.getBigInt64(32, true)) / 1000;
      offset = Math.max(offset, sent - arrived);
      const due = dueThere - offset;
      const playing = (view.getUint32(12, true) & 1) === 1;
      // Frames waiting from before a seek, a pause or a change of speed no longer show.
      if (view.getUint32(40, true) !== generation) {
        generation = view.getUint32(40, true);
        queue = [];
        anchor = null;
      }
      const lost = Math.abs(due - arrived) > MAX_WAIT_MS;
      if (lost) offset = sent - arrived;
      if (!playing || lost) queue = [{ buf, due: arrived, playing: false }];
      // A better guess of how the clocks differ can make this frame due before ones already waiting.
      else queue = [...queue.filter((f) => f.due < due), { buf, due, playing }];
      if (!raf) raf = requestAnimationFrame(tick);
    };

    const connect = () => {
      ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        retry = 0;
        setConnected(true);
      };
      ws.onmessage = (e) => receive(e.data as ArrayBuffer);
      ws.onclose = () => {
        setConnected(false);
        if (!closed) window.setTimeout(connect, Math.min(2000, 200 * ++retry));
      };
    };
    connect();
    return () => {
      closed = true;
      ws?.close();
      cancelAnimationFrame(raf);
    };
  }, [url, canvas]);
  return connected;
}

import { useEffect, useRef, useState } from "react";

const HEADER = 24;

/** Receives rendered frames from an engine and paints the newest one per display frame; `onFrame` gets its time. */
export function useFrameStream(url: string, canvas: React.RefObject<HTMLCanvasElement | null>, onFrame: (t: number, playing: boolean) => void) {
  const frame = useRef(onFrame);
  frame.current = onFrame;
  const [connected, setConnected] = useState(false);
  useEffect(() => {
    if (!url) return;
    let ws: WebSocket | null = null;
    let closed = false;
    let latest: ArrayBuffer | null = null;
    let raf = 0;
    let retry = 0;

    const draw = () => {
      raf = 0;
      const buf = latest;
      latest = null;
      const el = canvas.current;
      if (!buf || !el) return;
      const view = new DataView(buf);
      if (view.getUint32(0, true) !== 0x31465043) return; // "CPF1"
      const w = view.getUint32(4, true);
      const h = view.getUint32(8, true);
      const playing = (view.getUint32(12, true) & 1) === 1;
      const t = Number(view.getBigInt64(16, true));
      if (el.width !== w || el.height !== h) {
        el.width = w;
        el.height = h;
      }
      // Opaque: frames have no transparency, and an opaque canvas needs no blending with what is behind it.
      el.getContext("2d", { alpha: false })?.putImageData(new ImageData(new Uint8ClampedArray(buf, HEADER, w * h * 4), w, h), 0, 0);
      // The time of the frame on screen. After a seek the previous frame stays a moment; the UI flows wait for this.
      el.dataset.us = String(t);
      frame.current(t, playing);
    };

    const connect = () => {
      ws = new WebSocket(url);
      ws.binaryType = "arraybuffer";
      ws.onopen = () => {
        retry = 0;
        setConnected(true);
      };
      ws.onmessage = (e) => {
        latest = e.data as ArrayBuffer;
        if (!raf) raf = requestAnimationFrame(draw);
      };
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

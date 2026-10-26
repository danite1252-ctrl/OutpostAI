import { useEffect, useRef } from "react";

/**
 * The hands-free voice line: a few thin, overlapping waves that sit flat when it's quiet,
 * swell with the sound heard (or the reply being spoken), and breathe while it thinks.
 * Our own drawing (MIT); the same maths as the phone's src/wave.tsx.
 */
export type WaveMode = "idle" | "listening" | "heard" | "thinking" | "speaking";

const LINES = [
  { freq: 2.6, speed: 1.9, phase: 0.0, scale: 1.0, opacity: 0.95, width: 1.6 },
  { freq: 3.4, speed: -2.4, phase: 1.3, scale: 0.8, opacity: 0.55, width: 1.1 },
  { freq: 2.1, speed: 1.4, phase: 2.6, scale: 0.65, opacity: 0.45, width: 1.0 },
  { freq: 4.2, speed: 2.9, phase: 4.0, scale: 0.5, opacity: 0.35, width: 0.9 },
  { freq: 1.7, speed: -1.2, phase: 5.1, scale: 0.9, opacity: 0.3, width: 0.8 },
  { freq: 3.0, speed: 2.2, phase: 0.7, scale: 0.4, opacity: 0.25, width: 0.8 },
];
/** Each line twice, mirrored, so the waves open into soft lens shapes. */
const STROKES = LINES.flatMap((line) => [
  { ...line, sign: 1 },
  { ...line, sign: -1, opacity: line.opacity * 0.8 },
]);
const POINTS = 96;

/** Two soft humps that drift slowly, so the waves rise in places rather than all at once. */
function envelope(u: number, t: number) {
  const c1 = -0.38 + 0.12 * Math.sin(t * 0.37);
  const c2 = 0.34 + 0.12 * Math.sin(t * 0.29 + 1.7);
  const hump = (c: number, w: number) => Math.exp(-(((u - c) / w) ** 2));
  return (1 - u ** 8) * (0.15 + hump(c1, 0.28) + 0.85 * hump(c2, 0.24));
}

export function VoiceWave({
  level,
  mode,
  color = "#ffffff",
  height = 140,
}: {
  /** Read every frame: the loudness right now, 0..1. */
  level: () => number;
  mode: WaveMode;
  color?: string;
  height?: number;
}) {
  const canvas = useRef<HTMLCanvasElement | null>(null);
  const modeRef = useRef(mode);
  const colorRef = useRef(color);
  const levelRef = useRef(level);
  const pulse = useRef(0);
  if (modeRef.current !== mode) {
    // Something was heard: a quick swell, so it's clear it got it.
    if (mode === "heard") pulse.current = 1;
    modeRef.current = mode;
  }
  colorRef.current = color;
  levelRef.current = level;

  useEffect(() => {
    const el = canvas.current;
    if (!el) return;
    const ctx = el.getContext("2d");
    if (!ctx) return;
    let raf = 0;
    let shown = 0;
    let last = performance.now();
    const start = last;
    const draw = (now: number) => {
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const t = (now - start) / 1000;
      const dpr = window.devicePixelRatio || 1;
      const w = el.clientWidth;
      const h = el.clientHeight;
      if (el.width !== Math.round(w * dpr) || el.height !== Math.round(h * dpr)) {
        el.width = Math.round(w * dpr);
        el.height = Math.round(h * dpr);
      }
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
      ctx.clearRect(0, 0, w, h);

      const m = modeRef.current;
      let goal: number;
      if (m === "thinking") goal = 0.1 + 0.07 * Math.sin(t * 2.2);
      else if (m === "idle") goal = 0.02;
      else goal = 0.03 + Math.min(1, levelRef.current()) * 0.9;
      pulse.current = Math.max(0, pulse.current - dt * 2.2);
      goal = Math.max(goal, pulse.current * 0.55);
      // Rise fast, fall gently, like a meter.
      shown += (goal - shown) * Math.min(1, (goal > shown ? 14 : 4) * dt);
      const tt = t * (m === "thinking" ? 0.6 : 1);

      const mid = h / 2;
      ctx.strokeStyle = colorRef.current;
      ctx.lineCap = "round";
      STROKES.forEach((line, n) => {
        const i = n >> 1;
        ctx.globalAlpha = line.opacity;
        ctx.lineWidth = line.width;
        ctx.beginPath();
        for (let p = 0; p <= POINTS; p++) {
          const u = (p / POINTS) * 2 - 1;
          const shimmer = 0.55 + 0.45 * Math.sin(line.freq * Math.PI * u + tt * line.speed + line.phase);
          const a = (shown * line.scale + 0.012 * (i + 1)) * envelope(u, tt + i * 0.4);
          const x = (p / POINTS) * w;
          const y = mid + line.sign * shimmer * a * mid * 0.92;
          if (p === 0) ctx.moveTo(x, y);
          else ctx.lineTo(x, y);
        }
        ctx.stroke();
      });
      ctx.globalAlpha = 1;
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, []);

  return <canvas ref={canvas} aria-hidden="true" className="block w-full" style={{ height }} />;
}

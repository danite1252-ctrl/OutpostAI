import { useEffect, useRef, useState } from "react";
import { View } from "react-native";
import Svg, { Path } from "react-native-svg";

/**
 * The hands-free voice line: a few thin, overlapping waves that sit flat when it's quiet,
 * swell with the sound heard (or the reply being spoken), and breathe while it thinks.
 * Our own drawing (MIT); the same maths as the desktop's components/wave.tsx.
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
const POINTS = 72;

/** Two soft humps that drift slowly, so the waves rise in places rather than all at once. */
function envelope(u: number, t: number) {
  const c1 = -0.38 + 0.12 * Math.sin(t * 0.37);
  const c2 = 0.34 + 0.12 * Math.sin(t * 0.29 + 1.7);
  const hump = (c: number, w: number) => Math.exp(-(((u - c) / w) ** 2));
  // Pinned flat at both ends.
  const ends = 1 - u ** 8;
  return ends * (0.15 + hump(c1, 0.28) + 0.85 * hump(c2, 0.24));
}

/** Each line twice, mirrored, so the waves open into soft lens shapes. */
export const STROKES = LINES.flatMap((line) => [
  { ...line, sign: 1 },
  { ...line, sign: -1, opacity: line.opacity * 0.8 },
]);

export function wavePaths(t: number, amp: number, w: number, h: number): string[] {
  const mid = h / 2;
  return STROKES.map((line, n) => {
    const i = n >> 1;
    let d = "";
    for (let p = 0; p <= POINTS; p++) {
      const u = (p / POINTS) * 2 - 1;
      const shimmer = 0.55 + 0.45 * Math.sin(line.freq * Math.PI * u + t * line.speed + line.phase);
      // A hair of movement even when quiet, so it looks alive.
      const a = (amp * line.scale + 0.012 * (i + 1)) * envelope(u, t + i * 0.4);
      const y = mid + line.sign * shimmer * a * mid * 0.92;
      d += `${p === 0 ? "M" : "L"}${((p / POINTS) * w).toFixed(1)} ${y.toFixed(1)}`;
    }
    return d;
  });
}

export function VoiceWave({
  level,
  mode,
  color = "#ffffff",
  height = 120,
}: {
  /** 0..1, the loudness right now. */
  level: number;
  mode: WaveMode;
  color?: string;
  height?: number;
}) {
  const [width, setWidth] = useState(0);
  const [frame, setFrame] = useState<string[]>([]);
  const target = useRef(0);
  const shown = useRef(0);
  const modeRef = useRef(mode);
  const pulse = useRef(0);

  target.current = level;
  if (modeRef.current !== mode) {
    // Something was heard: a quick swell, so it's clear it got it.
    if (mode === "heard") pulse.current = 1;
    modeRef.current = mode;
  }

  useEffect(() => {
    if (!width) return;
    let raf = 0;
    let last = Date.now();
    const start = last;
    const tick = () => {
      const now = Date.now();
      const dt = Math.min(0.1, (now - last) / 1000);
      last = now;
      const t = (now - start) / 1000;
      const m = modeRef.current;
      let goal: number;
      if (m === "thinking") goal = 0.1 + 0.07 * Math.sin(t * 2.2);
      else if (m === "idle") goal = 0.02;
      else goal = 0.03 + Math.min(1, target.current) * 0.9;
      pulse.current = Math.max(0, pulse.current - dt * 2.2);
      goal = Math.max(goal, pulse.current * 0.55);
      // Rise fast, fall gently, like a meter.
      const k = goal > shown.current ? 14 : 4;
      shown.current += (goal - shown.current) * Math.min(1, k * dt);
      setFrame(wavePaths(t * (m === "thinking" ? 0.6 : 1), shown.current, width, height));
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [width, height]);

  return (
    <View style={{ alignSelf: "stretch", height }} onLayout={(e) => setWidth(Math.round(e.nativeEvent.layout.width))} accessible={false}>
      {width > 0 && (
        <Svg width={width} height={height}>
          {frame.map((d, i) => (
            <Path key={i} d={d} stroke={color} strokeOpacity={STROKES[i].opacity} strokeWidth={STROKES[i].width} fill="none" strokeLinecap="round" />
          ))}
        </Svg>
      )}
    </View>
  );
}

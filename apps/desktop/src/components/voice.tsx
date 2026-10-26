import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, streamMessage, voiceApi, type Persona } from "../api";
import { Button } from "./ui";
import { VoiceWave, type WaveMode } from "./wave";

function message(err: unknown) {
  if (err instanceof ApiError) return err.message;
  if (err instanceof DOMException && err.name === "NotAllowedError") return "Outpost isn't allowed to use the microphone.";
  if (err instanceof DOMException && err.name === "NotFoundError") return "No microphone found.";
  return err instanceof Error ? err.message : "Something went wrong.";
}

function blobToDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(new Error("The recording couldn't be read."));
    r.readAsDataURL(blob);
  });
}

/** One recording from the microphone, as a data URL the voice service can read. */
class Recorder {
  private chunks: Blob[] = [];
  private rec: MediaRecorder;
  constructor(stream: MediaStream) {
    const type = ["audio/webm;codecs=opus", "audio/webm", "audio/ogg;codecs=opus"].find((t) => MediaRecorder.isTypeSupported(t));
    this.rec = new MediaRecorder(stream, type ? { mimeType: type } : undefined);
    this.rec.ondataavailable = (e) => e.data.size > 0 && this.chunks.push(e.data);
    this.rec.start(250);
  }
  stop(): Promise<string> {
    return new Promise((resolve, reject) => {
      this.rec.onstop = () => {
        const blob = new Blob(this.chunks, { type: this.rec.mimeType || "audio/webm" });
        blobToDataUrl(blob).then(resolve, reject);
      };
      this.rec.stop();
    });
  }
  cancel() {
    this.rec.onstop = null;
    if (this.rec.state !== "inactive") this.rec.stop();
  }
}

/** Plays one thing at a time; starting another stops the last. */
const speaker = {
  audio: null as HTMLAudioElement | null,
  url: null as string | null,
  ctx: null as AudioContext | null,
  analyser: null as AnalyserNode | null,
  /** How loud what's playing is right now, 0..1 (for the hands-free wave). */
  level(): number {
    const a = this.analyser;
    if (!a || !this.audio) return 0;
    const buf = new Float32Array(a.fftSize);
    a.getFloatTimeDomainData(buf);
    let sum = 0;
    for (const v of buf) sum += v * v;
    return Math.min(1, Math.sqrt(sum / buf.length) * 4);
  },
  stop() {
    this.audio?.pause();
    if (this.url) URL.revokeObjectURL(this.url);
    this.audio = null;
    this.url = null;
  },
  /** Resolves when it finishes (or is stopped). */
  play(blob: Blob): Promise<void> {
    this.stop();
    const url = URL.createObjectURL(blob);
    const audio = new Audio(url);
    this.audio = audio;
    this.url = url;
    try {
      this.ctx ??= new AudioContext();
      if (!this.analyser) {
        this.analyser = this.ctx.createAnalyser();
        this.analyser.fftSize = 1024;
        this.analyser.connect(this.ctx.destination);
      }
      this.ctx.createMediaElementSource(audio).connect(this.analyser);
      void this.ctx.resume();
    } catch {
      // No meter; it still plays.
    }
    return new Promise((resolve) => {
      const done = () => {
        if (this.audio === audio) this.stop();
        resolve();
      };
      audio.onended = done;
      audio.onpause = done;
      audio.onerror = done;
      void audio.play().catch(done);
    });
  },
};

/** Speak text with a persona's voice. Resolves when it has been said. */
export async function sayAloud(token: string, text: string, persona: Persona | null) {
  const blob = await voiceApi.speak(token, text, { personaId: persona?.id });
  await speaker.play(blob);
}

export function stopSpeaking() {
  speaker.stop();
}

/** Click to talk, click again to stop; what was said goes to `onText`. */
export function MicButton({ token, disabled, onText, onError }: { token: string; disabled?: boolean; onText: (t: string) => void; onError: (m: string) => void }) {
  const [state, setState] = useState<"idle" | "listening" | "hearing">("idle");
  const rec = useRef<{ r: Recorder; stream: MediaStream } | null>(null);

  useEffect(
    () => () => {
      rec.current?.r.cancel();
      rec.current?.stream.getTracks().forEach((t) => t.stop());
    },
    [],
  );

  async function toggle() {
    if (state === "hearing") return;
    if (state === "idle") {
      try {
        const stream = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
        rec.current = { r: new Recorder(stream), stream };
        setState("listening");
      } catch (err) {
        onError(message(err));
      }
      return;
    }
    const current = rec.current;
    rec.current = null;
    if (!current) return setState("idle");
    setState("hearing");
    try {
      const audio = await current.r.stop();
      current.stream.getTracks().forEach((t) => t.stop());
      const t = await voiceApi.transcribe(token, audio);
      if (t.text) onText(t.text);
      else onError("Didn't catch that. Try again a little closer to the microphone.");
    } catch (err) {
      onError(message(err));
    } finally {
      setState("idle");
    }
  }

  const label = state === "listening" ? "Stop and use what I said" : state === "hearing" ? "Working out what you said" : "Talk instead of typing";
  return (
    <button
      type="button"
      onClick={() => void toggle()}
      disabled={disabled || state === "hearing"}
      aria-label={label}
      title={label}
      aria-pressed={state === "listening"}
      className={`flex min-h-11 min-w-11 items-center justify-center gap-2 rounded-xl border px-3 text-sm transition-colors disabled:opacity-50 ${
        state === "listening" ? "animate-pulse border-ember bg-ember/15 text-ember" : "border-line text-ink hover:bg-raised"
      }`}
    >
      <MicIcon />
      {state === "listening" ? "Listening… click to stop" : state === "hearing" ? "…" : null}
    </button>
  );
}

function MicIcon() {
  return (
    <svg width={18} height={18} viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.8} strokeLinecap="round" strokeLinejoin="round" aria-hidden="true">
      <path d="M12 3a3 3 0 0 0-3 3v6a3 3 0 0 0 6 0V6a3 3 0 0 0-3-3z M5 11a7 7 0 0 0 14 0 M12 18v3" />
    </svg>
  );
}

/** Read a reply aloud (or stop). */
export function PlayButton({ token, text, persona, onError }: { token: string; text: string; persona: Persona | null; onError: (m: string) => void }) {
  const [state, setState] = useState<"idle" | "loading" | "playing">("idle");
  async function toggle() {
    if (state !== "idle") {
      stopSpeaking();
      setState("idle");
      return;
    }
    setState("loading");
    try {
      const blob = await voiceApi.speak(token, text, { personaId: persona?.id });
      setState("playing");
      await speaker.play(blob);
    } catch (err) {
      onError(message(err));
    } finally {
      setState("idle");
    }
  }
  const label = state === "idle" ? "Read aloud" : state === "loading" ? "Getting the voice ready…" : "Stop reading";
  return (
    <button
      type="button"
      onClick={() => void toggle()}
      aria-label={label}
      title={label}
      className="rounded-md px-1.5 py-0.5 text-xs text-muted opacity-0 group-hover:opacity-100 hover:text-ink focus:opacity-100 data-[on=true]:opacity-100"
      data-on={state !== "idle"}
    >
      {state === "idle" ? "▶ Read aloud" : state === "loading" ? "…" : "■ Stop"}
    </button>
  );
}

type Phase = "starting" | "listening" | "hearing" | "thinking" | "speaking" | "paused";

/**
 * Hands-free: a call with the persona. It listens, notices when you stop talking,
 * sends what you said, speaks the reply, and listens again.
 */
export function HandsFree({
  token,
  persona,
  chatId,
  onTurn,
  onClose,
}: {
  token: string;
  persona: Persona | null;
  /** The chat to talk in (made on the first turn if there isn't one). */
  chatId: () => Promise<string>;
  /** A turn finished: reload the chat. */
  onTurn: () => void;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("starting");
  const [heard, setHeard] = useState("");
  const [reply, setReply] = useState("");
  const [error, setError] = useState<string | null>(null);
  const levelRef = useRef(0);
  const setLevel = (v: number) => {
    levelRef.current = v;
  };
  const [talking, setTalking] = useState(false);
  const [gotIt, setGotIt] = useState(false);
  const live = useRef(true);
  const phaseRef = useRef<Phase>("starting");
  const stream = useRef<MediaStream | null>(null);
  const ctx = useRef<AudioContext | null>(null);
  const analyser = useRef<AnalyserNode | null>(null);
  const recorder = useRef<Recorder | null>(null);

  const go = useCallback((p: Phase) => {
    phaseRef.current = p;
    setPhase(p);
  }, []);

  const end = useCallback(() => {
    live.current = false;
    recorder.current?.cancel();
    stopSpeaking();
    stream.current?.getTracks().forEach((t) => t.stop());
    void ctx.current?.close().catch(() => undefined);
    onClose();
  }, [onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && end();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [end]);

  // Loudness 0..1 from the analyser.
  const loudness = useCallback(() => {
    const a = analyser.current;
    if (!a) return 0;
    const buf = new Float32Array(a.fftSize);
    a.getFloatTimeDomainData(buf);
    let sum = 0;
    for (const v of buf) sum += v * v;
    return Math.sqrt(sum / buf.length);
  }, []);

  /** Record one thing said: starts on speech, ends after a pause. Empty if nothing. */
  const listenOnce = useCallback(async (): Promise<string> => {
    const s = stream.current!;
    // Learn the room's background level first.
    let floor = 0;
    for (let i = 0; i < 10; i++) {
      floor = Math.max(floor, loudness());
      await new Promise((r) => setTimeout(r, 30));
    }
    const threshold = Math.max(0.02, floor * 2.5);
    const rec = new Recorder(s);
    recorder.current = rec;
    const started = Date.now();
    let spoke = 0;
    let quietSince = 0;
    for (;;) {
      if (!live.current || phaseRef.current === "paused") {
        rec.cancel();
        return "";
      }
      const l = loudness();
      setLevel(Math.min(1, l / (threshold * 4)));
      const now = Date.now();
      if (l > threshold) {
        if (!spoke) {
          spoke = now;
          setTalking(true);
        }
        quietSince = 0;
      } else if (spoke && !quietSince) {
        quietSince = now;
      }
      // Done: talked for a moment, then quiet for 1.2 s; or 30 s of talking.
      if (spoke && quietSince && now - quietSince > 1200 && quietSince - spoke > 300) break;
      if (spoke && now - spoke > 30000) break;
      // Nothing said for 20 s: start a fresh recording so it stays small.
      if (!spoke && now - started > 20000) {
        rec.cancel();
        return listenOnce();
      }
      await new Promise((r) => setTimeout(r, 60));
    }
    setLevel(0);
    setTalking(false);
    setGotIt(true);
    window.setTimeout(() => setGotIt(false), 700);
    recorder.current = null;
    return rec.stop();
  }, [loudness]);

  const loop = useCallback(async () => {
    while (live.current) {
      if (phaseRef.current === "paused") {
        await new Promise((r) => setTimeout(r, 200));
        continue;
      }
      go("listening");
      const audio = await listenOnce();
      if (!live.current || !audio) continue;
      go("hearing");
      let text = "";
      try {
        text = (await voiceApi.transcribe(token, audio)).text;
      } catch (err) {
        setError(message(err));
        continue;
      }
      if (!text) continue;
      setHeard(text);
      setReply("");
      setError(null);
      go("thinking");
      let said = "";
      try {
        const id = await chatId();
        await streamMessage(token, id, text, [], {
          token: (t) => {
            said += t;
            setReply(said);
          },
          done: ({ message: m, error: e }) => {
            said = m.content;
            if (e) setError(e);
          },
          error: (m) => setError(m),
        });
      } catch (err) {
        setError(message(err));
      }
      onTurn();
      if (!live.current || !said.trim()) continue;
      go("speaking");
      try {
        await sayAloud(token, said, persona);
      } catch (err) {
        setError(message(err));
      }
    }
  }, [go, listenOnce, token, chatId, onTurn, persona]);

  useEffect(() => {
    live.current = true;
    (async () => {
      try {
        const s = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true } });
        stream.current = s;
        const c = new AudioContext();
        ctx.current = c;
        const a = c.createAnalyser();
        a.fftSize = 1024;
        c.createMediaStreamSource(s).connect(a);
        analyser.current = a;
        await loop();
      } catch (err) {
        setError(message(err));
        go("paused");
      }
    })();
    return () => {
      live.current = false;
    };
    // Start once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const who = persona?.name ?? "Outpost";
  const status =
    phase === "starting"
      ? "Starting the microphone…"
      : phase === "paused"
        ? "Paused"
        : gotIt
          ? "Got it"
          : talking
            ? "Listening…"
            : phase === "hearing" || phase === "thinking"
              ? "Thinking…"
              : phase === "speaking"
                ? `${who} is speaking`
                : "Go ahead, I'm listening";
  const mode: WaveMode =
    phase === "paused" || phase === "starting"
      ? "idle"
      : gotIt
        ? "heard"
        : talking
          ? "listening"
          : phase === "hearing" || phase === "thinking"
            ? "thinking"
            : phase === "speaking"
              ? "speaking"
              : "listening";
  const shown = reply.replace(/\[\[outpost-job:[^\]]+\]\]/g, "").trim();

  return (
    <div role="dialog" aria-modal="true" aria-label="Hands-free conversation" className="fixed inset-0 z-50 flex flex-col bg-black/95 p-8">
      <div className="flex items-center justify-center gap-2">
        <span className="inline-block size-3 rounded-full" style={{ background: persona?.avatar_color ?? "#FFB547" }} />
        <span className="font-medium text-soft">{who}</span>
      </div>
      <div className="mx-auto flex w-full max-w-3xl flex-1 flex-col justify-center gap-5 text-center">
        <VoiceWave
          mode={mode}
          // The persona speaks in its own colour; you're white.
          color={mode === "speaking" ? (persona?.avatar_color ?? "#FFB547") : "#ffffff"}
          level={() => (phaseRef.current === "speaking" ? speaker.level() : levelRef.current)}
          height={160}
        />
        <p className={`text-lg font-medium ${phase === "paused" ? "text-muted" : ""}`} aria-live="polite">
          {status}
        </p>
        {heard && <p className="line-clamp-2 text-sm text-muted">“{heard}”</p>}
        {shown && (phase === "thinking" || phase === "speaking") && <p className="mx-auto max-h-40 max-w-xl overflow-auto text-sm text-soft">{shown}</p>}
        {error && (
          <p className="text-sm text-danger" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="flex flex-col items-center gap-3">
        <div className="flex flex-wrap justify-center gap-3">
          {phase === "speaking" && (
            <Button variant="ghost" onClick={() => stopSpeaking()}>
              Interrupt
            </Button>
          )}
          {phase === "paused" ? (
            <Button variant="ghost" onClick={() => go("listening")} disabled={!stream.current}>
              Resume
            </Button>
          ) : (
            <Button variant="ghost" onClick={() => go("paused")}>
              Pause
            </Button>
          )}
          <Button variant="danger" onClick={end}>
            End
          </Button>
        </div>
        <p className="text-xs text-muted">Just talk, then pause: it sends after a moment of quiet. Esc ends.</p>
      </div>
    </div>
  );
}

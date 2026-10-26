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

/** Real words, not noise Whisper turned into a word or two of nothing. */
const NOT_SPEECH = /^\s*(you|thank you for watching|thanks for watching|\.+|…)[.!]?\s*$/i;
function isSpeech(text: string) {
  const t = text.trim();
  return t.replace(/[^A-Za-z0-9\u00C0-\uFFFF]/g, "").length >= 2 && !NOT_SPEECH.test(t);
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
  /** Paused because someone may be talking over it. */
  holding: false,
  /** Something is audible right now. */
  get sounding(): boolean {
    return !!this.audio && !this.audio.paused;
  },
  /** Pause without finishing; `resume` carries on. */
  hold() {
    if (this.audio && !this.audio.paused) {
      this.holding = true;
      this.audio.pause();
    }
  },
  resume() {
    if (this.audio && this.holding) {
      this.holding = false;
      void this.audio.play().catch(() => undefined);
    }
  },
  stop() {
    this.holding = false;
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
      audio.onpause = () => {
        if (!this.holding) done();
      };
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

/** A 16 kHz mono WAV, as a data URL for the voice service. */
function wavDataUrl(frames: Int16Array[]): string {
  const samples = frames.reduce((n, f) => n + f.length, 0);
  const buf = new ArrayBuffer(44 + samples * 2);
  const v = new DataView(buf);
  const text = (at: number, t: string) => [...t].forEach((c, i) => v.setUint8(at + i, c.charCodeAt(0)));
  text(0, "RIFF");
  v.setUint32(4, 36 + samples * 2, true);
  text(8, "WAVE");
  text(12, "fmt ");
  v.setUint32(16, 16, true);
  v.setUint16(20, 1, true);
  v.setUint16(22, 1, true);
  v.setUint32(24, 16000, true);
  v.setUint32(28, 32000, true);
  v.setUint16(32, 2, true);
  v.setUint16(34, 16, true);
  text(36, "data");
  v.setUint32(40, samples * 2, true);
  let at = 44;
  for (const f of frames) for (const x of f) {
    v.setInt16(at, x, true);
    at += 2;
  }
  const bytes = new Uint8Array(buf);
  let bin = "";
  for (let i = 0; i < bytes.length; i += 0x8000) bin += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return `data:audio/wav;base64,${btoa(bin)}`;
}

interface EarEvents {
  level: (level: number) => void;
  /** Someone started talking; `held` when that paused a reply. */
  start: (held: boolean) => void;
  /** One thing said. */
  utterance: (wav: string) => void;
  /** Only a cough or a bump. */
  discard: () => void;
}

/**
 * Hands-free listening, like a call: the microphone stays open (with echo cancelling) and
 * this notices when you start and stop talking, even while a reply plays. Talking over a
 * reply pauses it; the reply only stops for good once Whisper confirms real words.
 * The same rules as the phone's Conversation.kt.
 */
class Ear {
  muted = false;
  private ctx: AudioContext;
  private node: ScriptProcessorNode;
  private pending: number[] = [];
  private carry = 0;
  private acc = 0;
  private accN = 0;
  private floor = 0.01;
  private inSpeech = false;
  private loudRun = 0;
  private spoken = 0;
  private silent = 0;
  private tick = 0;
  private preRoll: Int16Array[] = [];
  private said: Int16Array[] = [];

  constructor(stream: MediaStream, private on: EarEvents, private talkOver: () => boolean) {
    this.ctx = new AudioContext();
    const src = this.ctx.createMediaStreamSource(stream);
    this.node = this.ctx.createScriptProcessor(2048, 1, 1);
    const ratio = this.ctx.sampleRate / 16000;
    this.node.onaudioprocess = (e) => {
      // Down to 16 kHz by averaging, then 20 ms frames.
      const input = e.inputBuffer.getChannelData(0);
      for (let i = 0; i < input.length; i++) {
        this.acc += input[i];
        this.accN++;
        this.carry++;
        if (this.carry >= ratio) {
          this.carry -= ratio;
          this.pending.push(this.acc / this.accN);
          this.acc = 0;
          this.accN = 0;
        }
      }
      while (this.pending.length >= 320) this.frame(this.pending.splice(0, 320));
    };
    const silence = this.ctx.createGain();
    silence.gain.value = 0;
    src.connect(this.node);
    this.node.connect(silence);
    silence.connect(this.ctx.destination);
    void this.ctx.resume();
  }

  close() {
    this.node.onaudioprocess = null;
    this.node.disconnect();
    void this.ctx.close().catch(() => undefined);
  }

  private frame(samples: number[]) {
    let sum = 0;
    let crossings = 0;
    const pcm = new Int16Array(samples.length);
    for (let i = 0; i < samples.length; i++) {
      const x = samples[i];
      sum += x * x;
      if (i > 0 && x >= 0 !== samples[i - 1] >= 0) crossings++;
      pcm[i] = Math.max(-32768, Math.min(32767, Math.round(x * 32767)));
    }
    const rms = Math.sqrt(sum / samples.length);
    // Voices cross zero at a moderate rate; hiss, clatter and fans much faster, hum much slower.
    const zcr = crossings / samples.length;
    const voiceLike = zcr >= 0.01 && zcr <= 0.32;
    const playing = speaker.sounding;
    if (!this.inSpeech && !playing) this.floor = rms < this.floor ? this.floor * 0.9 + rms * 0.1 : this.floor * 0.997 + rms * 0.003;
    const base = Math.max(this.floor * 3, 0.014);
    // While a reply plays a little of it gets past echo cancelling: need clearer speech.
    const threshold = playing ? base * 2.2 : base;
    const loud = rms > threshold && voiceLike;
    if (++this.tick % 5 === 0) this.on.level(Math.min(1, rms / (threshold * 3)));
    if (this.muted || (playing && !this.talkOver())) {
      this.inSpeech = false;
      this.said = [];
      this.preRoll = [];
      this.loudRun = 0;
      return;
    }
    if (!this.inSpeech) {
      this.preRoll.push(pcm);
      if (this.preRoll.length > 30) this.preRoll.shift();
      // Talking over a reply needs steady speech: most of the last ~0.4 s, not one bang.
      this.loudRun = loud ? this.loudRun + 1 : playing ? Math.max(0, this.loudRun - 2) : 0;
      if (this.loudRun >= (playing ? 18 : 4)) {
        this.inSpeech = true;
        this.said = this.preRoll; // Keep the start of the first word.
        this.preRoll = [];
        this.spoken = this.loudRun;
        this.silent = 0;
        if (playing) speaker.hold();
        this.on.start(playing);
      }
      return;
    }
    this.said.push(pcm);
    if (loud) {
      this.spoken++;
      this.silent = 0;
    } else this.silent++;
    // 0.8 s of quiet ends it (30 s at most); under 0.3 s of speech is a cough or a bump.
    if (this.silent >= 40 || this.said.length >= 1500) {
      this.inSpeech = false;
      this.loudRun = 0;
      const said = this.said;
      this.said = [];
      if (this.spoken >= 15) this.on.utterance(wavDataUrl(said));
      else this.on.discard();
    }
  }
}

/** Remembered while Outpost is open: whether talking over a reply stops it. */
let talkOverReplies = true;

/** The reply's own words, heard back through the microphone (not the user talking over it). */
function isEcho(heard: string, reply: string) {
  const words = (t: string) => t.toLowerCase().match(/[a-z0-9\u00C0-\uFFFF']{2,}/g) ?? [];
  const h = words(heard);
  if (h.length === 0) return false;
  const r = new Set(words(reply));
  return h.filter((w) => r.has(w)).length / h.length >= 0.6;
}

/** Something said: a recording to work out, or words already worked out. */
type Said = { audio: string } | { text: string };

/**
 * Hands-free: a call with the persona. The microphone stays open, it notices when you
 * stop talking, sends what you said, speaks the reply, and you can talk over the reply
 * to cut in (noise only pauses it for a moment).
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
  const [talking, setTalking] = useState(false);
  const [gotIt, setGotIt] = useState(false);
  const [over, setOver] = useState(talkOverReplies);
  const overRef = useRef(talkOverReplies);
  const levelRef = useRef(0);
  const phaseRef = useRef<Phase>("starting");
  /**
   * Which run of the conversation is current. React (in development) starts effects twice;
   * an older run sees the number change and stops, so only one is ever listening.
   */
  const run = useRef(0);
  const ear = useRef<Ear | null>(null);
  const queue = useRef<Said[]>([]);
  const busy = useRef(false);
  const held = useRef(false);
  /** The reply being read aloud, to tell its echo from you talking over it. */
  const speaking = useRef("");

  const go = useCallback((p: Phase) => {
    phaseRef.current = p;
    setPhase(p);
  }, []);

  const flashGotIt = useCallback(() => {
    setGotIt(true);
    window.setTimeout(() => setGotIt(false), 700);
  }, []);

  const end = useCallback(() => {
    run.current++;
    stopSpeaking();
    onClose();
  }, [onClose]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && end();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [end]);

  const toText = useCallback(async (item: Said) => ("text" in item ? item.text : (await voiceApi.transcribe(token, item.audio)).text.trim()), [token]);

  /** Work through what was said, one turn at a time. */
  const work = useCallback(
    async (me: number) => {
      if (busy.current) return;
      busy.current = true;
      const alive = () => run.current === me;
      try {
        while (alive() && queue.current.length) {
          go("hearing");
          let text = "";
          try {
            text = await toText(queue.current.shift()!);
          } catch (err) {
            setError(message(err));
            continue;
          }
          // Background noise that Whisper turned into nothing (or "you", "Thank you for watching"): ignore it.
          if (!alive() || !isSpeech(text)) continue;
          // Said more while this was being worked out: send it all together.
          while (queue.current.length && phaseRef.current !== "paused") {
            const more = await toText(queue.current.shift()!).catch(() => "");
            if (isSpeech(more)) text = `${text} ${more}`;
          }
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
          if (!alive() || !said.trim()) continue;
          // You spoke again while it was thinking: answer that instead (if it was really you).
          if (queue.current.length) {
            const pending = await Promise.all(queue.current.map((item) => toText(item).catch(() => "")));
            queue.current = pending.filter(isSpeech).map((t) => ({ text: t }));
            if (queue.current.length) continue;
          }
          go("speaking");
          try {
            speaking.current = said;
            await sayAloud(token, said, persona);
          } catch (err) {
            setError(message(err));
          }
        }
      } finally {
        busy.current = false;
        if (alive() && phaseRef.current !== "paused") go("listening");
      }
    },
    [chatId, go, onTurn, persona, toText, token],
  );

  // The turn loop as of this render (the microphone's callbacks were made on the first).
  const workRef = useRef(work);
  workRef.current = work;

  useEffect(() => {
    const me = ++run.current;
    const alive = () => run.current === me;
    let stream: MediaStream | null = null;
    let mine: Ear | null = null;
    (async () => {
      try {
        const s = await navigator.mediaDevices.getUserMedia({ audio: { echoCancellation: true, noiseSuppression: true, autoGainControl: true } });
        stream = s;
        if (!alive()) {
          s.getTracks().forEach((t) => t.stop());
          return;
        }
        mine = new Ear(
          s,
          {
            level: (l) => {
              levelRef.current = l;
            },
            start: (wasPlaying) => {
              if (!alive()) return;
              setTalking(true);
              if (wasPlaying) held.current = true;
            },
            discard: () => {
              setTalking(false);
              // Just a bump or a cough: carry on.
              if (held.current) {
                held.current = false;
                speaker.resume();
              }
            },
            utterance: (wav) => {
              if (!alive()) return;
              setTalking(false);
              if (phaseRef.current === "paused") return;
              if (held.current || phaseRef.current === "speaking") {
                // Talking over the reply: stop it only for real words, so noise never cuts it off.
                const wasHeld = held.current;
                held.current = false;
                voiceApi.transcribe(token, wav).then(
                  (r) => {
                    if (!alive()) return;
                    if (isSpeech(r.text) && !isEcho(r.text, speaking.current)) {
                      flashGotIt();
                      queue.current.push({ text: r.text.trim() });
                      stopSpeaking(); // The reading-aloud ends and the turn loop picks this up.
                      void workRef.current(me);
                    } else if (wasHeld) speaker.resume();
                  },
                  () => {
                    if (wasHeld) speaker.resume();
                  },
                );
                return;
              }
              flashGotIt();
              queue.current.push({ audio: wav });
              void workRef.current(me);
            },
          },
          () => overRef.current,
        );
        ear.current = mine;
        go("listening");
      } catch (err) {
        if (!alive()) return;
        setError(message(err));
        go("paused");
      }
    })();
    return () => {
      if (run.current === me) run.current++;
      (mine as Ear | null)?.close();
      (stream as MediaStream | null)?.getTracks().forEach((t) => t.stop());
      stopSpeaking();
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
          level={() => (mode === "speaking" ? speaker.level() : levelRef.current)}
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
          {phase === "paused" ? (
            <Button
              variant="ghost"
              onClick={() => {
                if (ear.current) ear.current.muted = false;
                setError(null);
                go("listening");
              }}
              disabled={!ear.current}
            >
              Resume
            </Button>
          ) : (
            <Button
              variant="ghost"
              onClick={() => {
                if (ear.current) ear.current.muted = true;
                queue.current = [];
                held.current = false;
                stopSpeaking();
                go("paused");
              }}
            >
              Pause
            </Button>
          )}
          <Button variant="danger" onClick={end}>
            End
          </Button>
        </div>
        <label className="flex items-center gap-2 text-sm text-soft">
          <input
            type="checkbox"
            checked={over}
            onChange={(e) => {
              talkOverReplies = e.target.checked;
              overRef.current = e.target.checked;
              setOver(e.target.checked);
            }}
          />
          Talk over replies to cut in
        </label>
        <p className="text-xs text-muted">Just talk, then pause: it sends after a moment of quiet. Esc ends.</p>
      </div>
    </div>
  );
}

/**
 * Voice on the phone. Listening uses the phone's own speech recognizer (on the phone
 * itself where Android can), so only text goes to the PC; a phone without one records
 * a clip for the PC's Whisper instead. Replies are spoken by the PC (Kokoro) and played here.
 */
import OutpostNet, { type RecordingEvent, type SpeechEvent } from "../modules/outpost-net";
import { ApiError, type Outpost } from "./outpost";

export interface VoiceSupport {
  /** The phone has a speech recognizer (otherwise the PC's Whisper listens). */
  recognizer: boolean;
  /** It works without the internet. */
  onDevice: boolean;
}

export function voiceSupport(): VoiceSupport {
  try {
    return OutpostNet.voiceSupport();
  } catch {
    return { recognizer: false, onDevice: false };
  }
}

let micAllowed = false;

/** Ask for the microphone the first time it's needed. */
export async function allowMicrophone(): Promise<void> {
  if (micAllowed) return;
  micAllowed = await OutpostNet.askMicrophone();
  if (!micAllowed) throw new ApiError(0, "microphone", "Outpost isn't allowed to use the microphone. Allow it in the phone's settings (Apps › Outpost › Permissions).");
}

export interface Listening {
  /** What was said; "" if nothing was. */
  done: Promise<string>;
  /** Finish now and use what was heard so far. */
  stop(): void;
  /** Stop and throw away what was heard. */
  cancel(): void;
}

/**
 * Hear one thing said. It finishes by itself after a pause (or on `stop()`).
 * `onPartial` gets the words as they're recognised (phone recognizer only).
 */
export function hear(
  api: Outpost,
  on: { partial?: (text: string) => void; level?: (level: number) => void; heard?: () => void } = {},
): Listening {
  const support = voiceSupport();
  let settle: { resolve: (t: string) => void; reject: (e: unknown) => void } | null = null;
  const done = new Promise<string>((resolve, reject) => (settle = { resolve, reject }));
  const finish = (fn: (s: NonNullable<typeof settle>) => void) => {
    const s = settle;
    settle = null;
    subs.forEach((x) => x.remove());
    if (s) fn(s);
  };
  const subs: { remove(): void }[] = [];

  if (support.recognizer) {
    subs.push(
      OutpostNet.addListener("onSpeech", (e: SpeechEvent) => {
        if (e.type === "level") on.level?.(e.level);
        else if (e.type === "partial") on.partial?.(e.text);
        else if (e.type === "final") finish((s) => s.resolve(e.text.trim()));
        else if (e.type === "error") finish((s) => s.reject(new ApiError(0, `speech_${e.code}`, e.message)));
      }),
    );
    void allowMicrophone().then(
      () => settle && OutpostNet.listen(null, true),
      (err) => finish((s) => s.reject(err)),
    );
    return {
      done,
      stop: () => OutpostNet.stopListening(),
      cancel: () => {
        OutpostNet.cancelListening();
        finish((s) => s.resolve(""));
      },
    };
  }

  // No recognizer on this phone: record, and let the PC's Whisper listen.
  subs.push(
    OutpostNet.addListener("onRecording", (e: RecordingEvent) => {
      if (e.type === "level") on.level?.(e.level);
      else if (e.type === "error") finish((s) => s.reject(new ApiError(0, "microphone", e.message)));
      else if (e.type === "done") {
        if (!e.audio) return finish((s) => s.resolve(""));
        on.heard?.();
        const audio = e.audio;
        finish((s) =>
          api.transcribe(audio).then(
            (r) => s.resolve(r.text.trim()),
            (err) => s.reject(err),
          ),
        );
      }
    }),
  );
  void allowMicrophone().then(
    () => settle && OutpostNet.record(true),
    (err) => finish((s) => s.reject(err)),
  );
  return {
    done,
    stop: () => OutpostNet.stopRecording(),
    cancel: () => {
      OutpostNet.cancelRecording();
      finish((s) => s.resolve(""));
    },
  };
}

// ------------------------------------------------------------------ speaking

/** Reply text as pieces to speak: no picture cards or code, split at sentences, first piece short. */
export function speechParts(text: string): string[] {
  const clean = text
    .replace(/\[\[outpost-job:[^\]]+\]\]/g, " ")
    .replace(/```[\s\S]*?(```|$)/g, " ")
    .replace(/\s+/g, " ")
    .trim();
  if (!clean) return [];
  const sentences = clean.match(/[^.!?…]+(?:[.!?…]+["')\]]*|$)\s*/g) ?? [clean];
  const parts: string[] = [];
  let cur = "";
  for (const sentence of sentences) {
    // Start talking quickly: the first piece is about one sentence; later ones are longer.
    const limit = parts.length === 0 ? 120 : 400;
    if (cur && (cur + sentence).length > limit) {
      parts.push(cur.trim());
      cur = "";
    }
    cur += sentence;
  }
  if (cur.trim()) parts.push(cur.trim());
  return parts.filter((p) => /[A-Za-z0-9\u00C0-\uFFFF]/.test(p));
}

type Run = { cancelled: boolean; key: string; wake?: () => void };
let current: Run | null = null;
const watchers = new Set<(key: string | null) => void>();
let clips = 0;

function announce() {
  const key = current?.key ?? null;
  watchers.forEach((fn) => fn(key));
}

/** Follow what's being read aloud (its key), or null. */
export function onSaying(fn: (key: string | null) => void): () => void {
  watchers.add(fn);
  return () => watchers.delete(fn);
}

function playClip(uri: string, run: Run): Promise<void> {
  return new Promise((resolve, reject) => {
    const sub = OutpostNet.addListener("onPlayback", (e) => {
      if (e.type === "level") return;
      sub.remove();
      run.wake = undefined;
      if (e.type === "error") reject(new ApiError(0, "playback", e.message ?? "The reply couldn't be played."));
      else resolve();
    });
    run.wake = () => {
      sub.remove();
      resolve();
    };
    OutpostNet.play(uri);
  });
}

/**
 * Read `text` aloud in the persona's voice. Resolves when it's finished (or stopped).
 * The next piece is fetched from the PC while the current one plays.
 */
export async function say(api: Outpost, text: string, personaId: string | null, key = "say"): Promise<void> {
  stopSaying();
  const parts = speechParts(text);
  if (parts.length === 0) return;
  const run: Run = { cancelled: false, key };
  current = run;
  announce();
  const fetchPart = (i: number) => {
    const p = api.speech(parts[i], personaId, `say-${Date.now()}-${++clips}.wav`);
    p.catch(() => undefined); // Handled when awaited; this stops "unhandled" warnings after a stop.
    return p;
  };
  try {
    let next = fetchPart(0);
    for (let i = 0; i < parts.length; i++) {
      const uri = await next;
      if (run.cancelled) return;
      if (i + 1 < parts.length) next = fetchPart(i + 1);
      await playClip(uri, run);
      if (run.cancelled) return;
    }
  } finally {
    if (current === run) {
      current = null;
      announce();
    }
  }
}

export function stopSaying() {
  const run = current;
  if (!run) return;
  run.cancelled = true;
  current = null;
  OutpostNet.stopPlaying();
  run.wake?.();
  announce();
}

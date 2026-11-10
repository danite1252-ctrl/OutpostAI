import { NativeModule, requireNativeModule } from "expo";

/** One HTTPS request to the paired Outpost; only its pinned certificate is accepted. */
export interface WakeStatus {
  on: boolean;
  word: string;
  persona: string | null;
  threshold: number;
  /** When it started listening (ms). */
  since: number;
  batteryStart: number;
  batteryNow: number;
  charging: boolean;
  wakes: number;
  /** Times the phone thought it heard the word but the Outpost found it wasn't said (TV...). */
  ignored?: number;
  lastWake: number;
  error: string | null;
  words: { id: string; name: string }[];
}

export interface RequestOptions {
  url: string;
  method?: string;
  headers?: Record<string, string>;
  body?: string | null;
  /** SHA-256 of the Outpost's certificate, lowercase hex. */
  fingerprint: string;
  /** Return the body as base64 (pictures). */
  base64?: boolean;
  timeoutMs?: number;
}

export interface NativeResponse {
  status: number;
  body: string;
  contentType: string;
}

/** An Outpost found on the Wi-Fi. */
export interface FoundService {
  name: string;
  host: string;
  port: number;
  txt: Record<string, string>;
}

type Events = {
  onChunk(e: { id: string; line: string }): void;
  onEnd(e: { id: string; status: number; body: string }): void;
  onError(e: { id: string; message: string }): void;
  onService(e: FoundService): void;
  onServiceLost(e: { name: string }): void;
  onSpeech(e: SpeechEvent): void;
  onRecording(e: RecordingEvent): void;
  onPlayback(e: { type: "end" | "stopped" | "error" | "level"; message?: string; level?: number }): void;
  onConverse(e: ConverseEvent): void;
  onUpdate(e: UpdateEvent): void;
  /** Something was shared with Outpost while it was open: call takeShared(). */
  onShare(e: Record<string, never>): void;
};

/** A file another app shared, copied into Outpost's cache. */
export interface SharedFile {
  /** Its path on the phone (absent if it couldn't be taken, see error). */
  path?: string;
  name: string;
  mime?: string;
  size?: number;
  /** For a picture: a smaller JPEG, ready for chat. */
  dataUri?: string | null;
  /** e.g. "too big". */
  error?: string;
}

/** What was shared with Outpost, or an Outpost link that was opened. */
export interface Shared {
  text?: string | null;
  subject?: string | null;
  files?: SharedFile[];
  /** An opened link (a pairing QR scanned with the camera). */
  link?: string;
}

/** Installing a phone app update: download progress, Android's prompt, the result. */
export type UpdateEvent =
  | { type: "progress"; percent: number }
  | { type: "confirm" | "installed" | "cancelled" }
  | { type: "failed"; message: string };

/** Hands-free listening (always-on microphone with echo cancelling). */
export type ConverseEvent =
  | { type: "ready"; echoCancelling: boolean }
  | { type: "level"; level: number; db?: number; roomDb?: number; voiceLike?: boolean; inSpeech?: boolean }
  /** Someone started talking; `interrupted` when that paused a reply (resume it if it was noise). */
  | { type: "start"; interrupted: boolean }
  /** One thing said, as a base64 16 kHz WAV. */
  | { type: "utterance"; audio: string }
  /** It was only a cough or a bump. */
  | { type: "discard" }
  | { type: "error"; message: string };

/** The phone's speech recognizer. `final` with empty text means nothing was heard. */
export type SpeechEvent =
  | { type: "ready" | "speaking" }
  | { type: "level"; level: number }
  | { type: "partial" | "final"; text: string }
  | { type: "error"; code: number; message: string };

/** A clip recorded for the PC's Whisper; `done` carries it as base64 M4A ("" if nothing was said). */
export type RecordingEvent =
  | { type: "level"; level: number }
  | { type: "done"; audio: string }
  | { type: "error"; message: string };

declare class OutpostNetModule extends NativeModule<Events> {
  /** The certificate fingerprint an address presents, before anything is trusted. */
  fingerprint(host: string, port: number): Promise<string>;
  request(options: RequestOptions): Promise<NativeResponse>;
  /** Save the reply to the app's cache as `name`; resolves with a file:// URI. */
  download(options: RequestOptions, name: string): Promise<string>;
  /** Streamed reply: lines arrive as onChunk, then onEnd (or onError). */
  stream(id: string, options: RequestOptions): void;
  cancel(id: string): void;
  startDiscovery(): void;
  /** Whether the phone has a speech recognizer, and whether it works without the internet. */
  voiceSupport(): { recognizer: boolean; onDevice: boolean };
  askMicrophone(): Promise<boolean>;
  /** Listen with the phone's recognizer: onSpeech events, ending with `final` or `error`. */
  listen(language: string | null, partial: boolean): void;
  stopListening(): void;
  cancelListening(): void;
  /** Record for Whisper; with autoStop it ends itself after a pause. Ends with onRecording `done`. */
  record(autoStop: boolean): void;
  stopRecording(): void;
  cancelRecording(): void;
  /** Play a file:// clip (deleted afterwards); onPlayback when it ends. */
  play(uri: string): void;
  stopPlaying(): void;
  /** Carry on with a reply that talking-over paused (it was only noise). */
  resumePlaying(): void;
  /** Open the microphone for a hands-free conversation; with bargeIn, talking stops a reply. */
  converse(bargeIn: boolean): void;
  converseMuted(muted: boolean): void;
  /** Waiting for a yes or no: short answers count. */
  converseBrief?(brief: boolean): void;
  /** The wake word: listen for it (a foreground service, with a notification and Stop), even with
   *  the screen off; what's said after it goes to the persona and the answer is spoken. */
  wakeStart?(word: string, personaId: string, threshold: number, hosts: string[], port: number, fingerprint: string, token: string): void;
  wakeStop?(): void;
  /** Turn it back on if it was on (not switched off) and isn't listening: when the app opens. */
  wakeResume?(hosts: string[], port: number, fingerprint: string, token: string): void;
  wakeStatus?(): WakeStatus;
  /** How loud Outpost speaks with the app closed: 0 = the phone's media volume, else at least this %. */
  voiceLevel?(): number;
  /** Routine news set to be said on the phone is said on this one (on unless turned off). */
  sayNews?(): boolean;
  setSayNews?(on: boolean): void;
  setVoiceLevel?(percent: number): void;
  /** Play the wake word's "listening" chime, at that level. */
  voiceChime?(): Promise<void>;
  /** The detector on a recording of "Alexa" and of other words: it works on this phone. */
  wakeCheck?(): Promise<{ heard: number; other: number; ok: boolean }>;
  stopConversation(): void;
  /** Whether Outpost may install apps, and whether its own updates can skip the prompt. */
  updateStatus(): { canInstall: boolean; silent: boolean; sdk: number };
  /** Android's "Install unknown apps" page for Outpost. */
  openInstallSettings(): void;
  /** The path of an update already downloaded under `name`, or null. */
  updateDownloaded(name: string): string | null;
  /** A small text file over ordinary HTTPS (GitHub's latest.json). */
  fetchText(url: string): Promise<string>;
  /** Download an update (HTTPS); resolves with its path. `mirror`: the same file on Outpost's
   *  own download server, tried first (held to `sha256`), with `url` when that fails. */
  downloadUpdate(url: string, name: string, sha256: string | null, mirror: string | null): Promise<string>;
  /** Hand a downloaded update to Android's installer (it closes Outpost to update). */
  installUpdate(path: string): Promise<void>;
  stopDiscovery(): void;
  /** What was shared with Outpost (or a link that opened it), once; null if nothing. */
  takeShared(): Promise<Shared | null>;
  /** Delete the copies of shared files. */
  clearShared(): void;
  /** Listen for a sign-in's browser callback at http://127.0.0.1:port/callback; false if taken. */
  signInListen(port: number): boolean;
  /** The callback's path and query once the browser comes back (brings Outpost to the front). */
  signInWait(timeoutMs: number): Promise<string>;
  /** Stop waiting for a sign-in. */
  signInStop(): void;
  /** Copy a downloaded file (file:// or path) into Pictures, Movies or Download, in an Outpost
   * folder; resolves with its content:// URI. Android 10 or newer. */
  saveToPhone(path: string, name: string, mime: string): Promise<string>;
  /** Android's share sheet for a file saved with saveToPhone. */
  shareSaved(uri: string, mime: string, title: string): Promise<void>;
  /** Check the Outpost for finished jobs in the background (WorkManager) and notify about long
   * ones; `on` false stops it and forgets the connection. */
  notifyConfigure(on: boolean, hosts: string[], port: number, fingerprint: string, token: string): void;
  /** Jobs of mine are running: watch for them finishing, straight away (a quiet ongoing
   * notification while it does; carries on after the app is left). */
  watchJobs(): void;
  /** A file's size in bytes. */
  fileSize(path: string): number;
  /** Part of a file as base64. */
  readChunk(path: string, offset: number, length: number): Promise<string>;
  /** The app came to the front, or was left (no notifications while it's in front). */
  notifyInFront(inFront: boolean): void;
  /** Android 13+: ask to show notifications; true when allowed. */
  askNotifications(): Promise<boolean>;
  /** POST a file from the phone as the body. */
  upload(options: RequestOptions, path: string, contentType: string): Promise<NativeResponse>;
}

export default requireNativeModule<OutpostNetModule>("OutpostNet");

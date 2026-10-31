import { NativeModule, requireNativeModule } from "expo";

/** One HTTPS request to the paired Outpost; only its pinned certificate is accepted. */
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
  stopConversation(): void;
  /** Whether Outpost may install apps, and whether its own updates can skip the prompt. */
  updateStatus(): { canInstall: boolean; silent: boolean; sdk: number };
  /** Android's "Install unknown apps" page for Outpost. */
  openInstallSettings(): void;
  /** The path of an update already downloaded under `name`, or null. */
  updateDownloaded(name: string): string | null;
  /** A small text file over ordinary HTTPS (GitHub's latest.json). */
  fetchText(url: string): Promise<string>;
  /** Download an update (HTTPS); resolves with its path. */
  downloadUpdate(url: string, name: string, sha256: string | null): Promise<string>;
  /** Hand a downloaded update to Android's installer (it closes Outpost to update). */
  installUpdate(path: string): Promise<void>;
  stopDiscovery(): void;
  /** What was shared with Outpost (or a link that opened it), once; null if nothing. */
  takeShared(): Promise<Shared | null>;
  /** Delete the copies of shared files. */
  clearShared(): void;
  /** Copy a downloaded file (file:// or path) into Pictures, Movies or Download, in an Outpost
   * folder; resolves with its content:// URI. Android 10 or newer. */
  saveToPhone(path: string, name: string, mime: string): Promise<string>;
  /** Android's share sheet for a file saved with saveToPhone. */
  shareSaved(uri: string, mime: string, title: string): Promise<void>;
  /** Check the Outpost for finished jobs in the background (WorkManager) and notify about long
   * ones; `on` false stops it and forgets the connection. */
  notifyConfigure(on: boolean, hosts: string[], port: number, fingerprint: string, token: string): void;
  /** The app came to the front, or was left (no notifications while it's in front). */
  notifyInFront(inFront: boolean): void;
  /** Android 13+: ask to show notifications; true when allowed. */
  askNotifications(): Promise<boolean>;
  /** POST a file from the phone as the body. */
  upload(options: RequestOptions, path: string, contentType: string): Promise<NativeResponse>;
}

export default requireNativeModule<OutpostNetModule>("OutpostNet");

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
};

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
  stopDiscovery(): void;
}

export default requireNativeModule<OutpostNetModule>("OutpostNet");

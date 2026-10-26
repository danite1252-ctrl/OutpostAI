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
};

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
  stopDiscovery(): void;
}

export default requireNativeModule<OutpostNetModule>("OutpostNet");

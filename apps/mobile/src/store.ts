/** The paired Outpost, kept in the phone's secure storage (Android Keystore). */
import * as SecureStore from "expo-secure-store";
import type { Account, Connection } from "./outpost";

export interface Saved extends Connection {
  token: string;
  account: Account;
  sessionId: string;
  device: string;
}

const KEY = "outpost.pairing.v1";

export async function load(): Promise<Saved | null> {
  try {
    const raw = await SecureStore.getItemAsync(KEY);
    return raw ? (JSON.parse(raw) as Saved) : null;
  } catch {
    return null;
  }
}

export async function save(s: Saved): Promise<void> {
  await SecureStore.setItemAsync(KEY, JSON.stringify(s));
}

export async function forget(): Promise<void> {
  await SecureStore.deleteItemAsync(KEY);
}

const VOICE_CHECK_KEY = "outpost.voice-check.v1";

/** Troubleshooting: show microphone levels on the hands-free screen. Off unless turned on in You. */
export async function loadVoiceCheck(): Promise<boolean> {
  try {
    return (await SecureStore.getItemAsync(VOICE_CHECK_KEY)) === "1";
  } catch {
    return false;
  }
}

export async function saveVoiceCheck(on: boolean): Promise<void> {
  try {
    await SecureStore.setItemAsync(VOICE_CHECK_KEY, on ? "1" : "0");
  } catch {
    // Not kept: it goes back to off, which is the default anyway.
  }
}

const AWAY_KEY = "outpost.away.v1";

/** Note when the app went to the background (kept across the app being closed). */
export async function markAway(): Promise<void> {
  try {
    await SecureStore.setItemAsync(AWAY_KEY, String(Date.now()));
  } catch {
    // Not kept: the app asks to unlock next time, which is the safe side.
  }
}

/** How long ago the app went to the background; very long if unknown. */
export async function awayFor(): Promise<number> {
  try {
    const t = Number(await SecureStore.getItemAsync(AWAY_KEY));
    return t > 0 ? Math.max(0, Date.now() - t) : Number.MAX_SAFE_INTEGER;
  } catch {
    return Number.MAX_SAFE_INTEGER;
  }
}

/** Parse the QR link the PC shows: outpost://pair?v=1&name=..&hosts=..&port=..&fp=..&code=.. */
export function parsePairLink(link: string): { name: string; hosts: string[]; port: number; fingerprint: string; code: string } | null {
  if (!link.startsWith("outpost://pair?")) return null;
  const q = new Map<string, string>();
  for (const part of link.slice("outpost://pair?".length).split("&")) {
    const [k, v = ""] = part.split("=");
    try {
      q.set(k, decodeURIComponent(v));
    } catch {
      return null;
    }
  }
  const port = Number(q.get("port"));
  const fingerprint = (q.get("fp") ?? "").toLowerCase();
  const code = q.get("code") ?? "";
  const hosts = (q.get("hosts") ?? "").split(",").filter(Boolean);
  if (!port || !/^[0-9a-f]{64}$/.test(fingerprint) || !/^\d{6}$/.test(code) || hosts.length === 0) return null;
  return { name: q.get("name") ?? "Outpost", hosts, port, fingerprint, code };
}

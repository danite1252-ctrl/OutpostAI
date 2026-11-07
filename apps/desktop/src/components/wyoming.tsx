// Settings › Voice › Home Assistant: Outpost's listening and voices offered to Home Assistant
// (its Wyoming protocol), for voice devices around the house. Off until an owner turns it on;
// then only this home network (or the addresses listed) may use it.

import { useEffect, useState } from "react";
import { ApiError, voiceApi, type WyomingStatus } from "../api";
import { Button, ErrorNote, Toggle } from "./ui";

function message(e: unknown) {
  return e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
}

export function HomeAssistantVoice({ token }: { token: string }) {
  const [s, setS] = useState<WyomingStatus | null>(null);
  const [port, setPort] = useState("");
  const [allow, setAllow] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    voiceApi.wyoming(token).then(
      (w) => {
        setS(w);
        setPort(String(w.port));
        setAllow(w.allow.join(", "));
      },
      (e) => setError(message(e)),
    );
  }, [token]);

  async function save(on: boolean) {
    setError(null);
    setBusy(true);
    try {
      const list = allow
        .split(/[\s,]+/)
        .map((a) => a.trim())
        .filter(Boolean);
      const w = await voiceApi.setWyoming(token, { on, port: Number(port) || 10700, allow: list });
      setS(w);
      setPort(String(w.port));
      setAllow(w.allow.join(", "));
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  if (!s) return error ? <ErrorNote>{error}</ErrorNote> : null;
  const changed = String(s.port) !== port.trim() || s.allow.join(", ") !== allow.trim();
  const field = "min-h-9 rounded-lg border border-line bg-surface px-3 text-sm text-ink";
  return (
    <div className="mt-6 flex flex-col gap-3 border-t border-line pt-5">
      <div className="flex items-center gap-3">
        <div className="flex-1">
          <h3 className="font-medium">Home Assistant voice</h3>
          <p className="text-sm text-muted">
            Offer Outpost's listening and voices to Home Assistant, for its voice devices around the house.
          </p>
        </div>
        <Toggle checked={s.on} label="Offer voice to Home Assistant" onChange={(on) => void save(on)} disabled={busy} />
      </div>
      <div className="flex flex-wrap items-end gap-3 text-sm">
        <label className="flex flex-col gap-1">
          <span className="text-muted">Port</span>
          <input className={`${field} w-28`} inputMode="numeric" value={port} onChange={(e) => setPort(e.target.value)} />
        </label>
        <label className="flex min-w-64 flex-1 flex-col gap-1">
          <span className="text-muted">Only from (Home Assistant's address; empty: anything on this home network)</span>
          <input className={field} placeholder="192.168.1.20" value={allow} onChange={(e) => setAllow(e.target.value)} />
        </label>
        {changed && (
          <Button variant="ghost" disabled={busy} onClick={() => void save(s.on)}>
            Save
          </Button>
        )}
      </div>
      {s.on && s.running && (
        <p className="rounded-xl bg-raised px-4 py-3 text-sm">
          In Home Assistant: Settings › Devices &amp; services › Add integration › <b>Wyoming Protocol</b>, host{" "}
          <span className="font-mono">{s.host ?? "this server's address"}</span>, port <span className="font-mono">{s.port}</span> (it may
          also find it by itself). Then in Settings › Voice assistants, choose <b>Outpost speech-to-text</b> and an <b>Outpost voice</b>.
        </p>
      )}
      {s.on && !s.running && <ErrorNote>{s.problem ?? "It isn't running. Try turning it off and on."}</ErrorNote>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

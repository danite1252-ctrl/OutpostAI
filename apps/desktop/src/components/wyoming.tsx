// Settings › Voice › Home Assistant: Outpost's listening and voices offered to Home Assistant
// (its Wyoming protocol), for voice devices around the house. Off until an owner turns it on;
// then only this home network (or the addresses listed) may use it.

import { useEffect, useState } from "react";
import { api, ApiError, voiceApi, type Persona, type WyomingStatus } from "../api";
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
  const [personas, setPersonas] = useState<Persona[]>([]);

  useEffect(() => {
    api.personas(token).then(setPersonas, () => undefined);
  }, [token]);

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

  async function save(on: boolean, agentPersona?: string | null) {
    setError(null);
    setBusy(true);
    try {
      const list = allow
        .split(/[\s,]+/)
        .map((a) => a.trim())
        .filter(Boolean);
      const persona = agentPersona === undefined ? (s?.agent?.persona_id ?? null) : agentPersona;
      const w = await voiceApi.setWyoming(token, {
        on,
        port: Number(port) || 10700,
        allow: list,
        agent: persona ? { account_id: "", persona_id: persona } : null,
      });
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
      <label className="flex flex-wrap items-center gap-3 text-sm">
        <span className="text-muted">Answer as Home Assistant's conversation agent, as you, with</span>
        <select
          aria-label="Persona that answers"
          className={field}
          value={s.agent?.persona_id ?? ""}
          disabled={busy}
          onChange={(e) => void save(s.on, e.target.value || null)}
        >
          <option value="">Nobody (listening and voices only)</option>
          {personas.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      {s.agent && (
        <p className="text-xs text-muted">
          It uses your tools and rules. Anything that asks first (locks, alarms, doors, actions set to ask) is asked aloud: say yes or
          no. The conversation is kept in a chat called "Home Assistant voice". Only Home Assistant's address (above) can use it.
        </p>
      )}
      {s.on && s.running && (
        <p className="rounded-xl bg-raised px-4 py-3 text-sm">
          In Home Assistant: Settings › Devices &amp; services › Add integration › <b>Wyoming Protocol</b>, host{" "}
          <span className="font-mono">{s.host ?? "this server's address"}</span>, port <span className="font-mono">{s.port}</span> (it may
          also find it by itself). Then in Settings › Voice assistants, choose <b>Outpost speech-to-text</b> and an <b>Outpost voice</b>
          {s.agent ? (
            <>
              , and <b>Outpost</b> as the conversation agent
            </>
          ) : null}
          .
        </p>
      )}
      {s.on && !s.running && <ErrorNote>{s.problem ?? "It isn't running. Try turning it off and on."}</ErrorNote>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

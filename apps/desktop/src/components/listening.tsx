// Settings › Voice: which model listens (speech to text). Bigger hears better and replies
// a little slower; one that isn't downloaded is fetched first, and the current one carries
// on until it's there.

import { useCallback, useEffect, useState } from "react";
import { ApiError, voiceApi, type VoiceStatus } from "../api";
import { ErrorNote } from "./ui";

const size = (mb: number) => (mb >= 1000 ? `${(mb / 1000).toFixed(1)} GB` : `${mb} MB`);

export function ListeningPanel({ token, canManage }: { token: string; canManage: boolean }) {
  const [status, setStatus] = useState<VoiceStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => voiceApi.status(token).then(setStatus, () => undefined), [token]);
  useEffect(() => void load(), [load]);
  const fetching = status?.listening_fetch?.model ?? null;
  useEffect(() => {
    if (!fetching) return;
    const t = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(t);
  }, [fetching, load]);

  if (!status?.listening_models) return null;
  const choose = (id: string) => {
    setError(null);
    voiceApi.setListening(token, id).then(setStatus, (e) => setError(e instanceof ApiError ? e.message : "Something went wrong."));
  };
  const failed = status.listening_fetch?.failed;
  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5">
      <div>
        <h3 className="font-semibold">Listening</h3>
        <p className="text-sm text-muted">
          The model that turns what you say into text (the microphone button, hands-free and the phone). It runs on the
          processor, so it never takes the graphics card from chat or pictures.
        </p>
      </div>
      <div className="flex flex-col gap-2" role="radiogroup" aria-label="Listening model">
        {status.listening_models.map((m) => {
          const chosen = status.listening === m.id;
          const inUse = status.listening_in_use === m.id;
          const downloading = fetching === m.id;
          return (
            <label
              key={m.id}
              className={`flex cursor-pointer items-start gap-3 rounded-xl border p-3 ${chosen ? "border-ember bg-raised" : "border-line"} ${
                canManage ? "" : "cursor-default"
              }`}
            >
              <input
                type="radio"
                name="listening"
                className="mt-1 accent-ember"
                checked={chosen}
                disabled={!canManage || (!!fetching && !downloading)}
                onChange={() => choose(m.id)}
              />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="text-sm font-medium">
                  {m.label}
                  {inUse && <span className="ml-2 text-xs font-normal text-ok">In use</span>}
                </span>
                <span className="text-xs text-muted">{m.description}</span>
                <span className="text-xs text-muted" aria-live={downloading ? "polite" : undefined}>
                  {downloading
                    ? `Downloading ${size(m.size_mb)}… ${status.listening_fetch?.progress != null ? `${Math.round(status.listening_fetch.progress * 100)}%` : ""} (${
                        status.listening_models?.find((x) => x.id === status.listening_in_use)?.label ?? "the current one"
                      } carries on until it's done)`
                    : m.downloaded
                      ? "Downloaded"
                      : `${size(m.size_mb)} to download`}
                </span>
                {failed === m.id && status.listening_fetch?.error && !downloading && (
                  <span className="text-xs text-danger">The download failed: {status.listening_fetch.error}. Choose it again to retry.</span>
                )}
              </span>
            </label>
          );
        })}
      </div>
      {!canManage && <p className="text-xs text-muted">Only an owner can change this.</p>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

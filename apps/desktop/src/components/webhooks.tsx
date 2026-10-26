import { useCallback, useEffect, useState } from "react";
import { ApiError, hooksApi, type Webhook } from "../api";
import { ago } from "./context";
import { Button, ErrorNote, Field } from "./ui";

const LABELS: Record<string, string> = {
  "job.finished": "A picture or video is done",
  "training.finished": "A training is done",
  "character.created": "A character is added",
};

/** Your URLs that Outpost tells when things finish (n8n, Home Assistant, scripts). */
export function WebhooksPanel({ token }: { token: string }) {
  const [hooks, setHooks] = useState<Webhook[]>([]);
  const [events, setEvents] = useState<string[]>([]);
  const [url, setUrl] = useState("");
  const [picked, setPicked] = useState<string[]>(["job.finished"]);
  const [secret, setSecret] = useState<string | null>(null);
  const [tested, setTested] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fail = (err: unknown) => setError(err instanceof ApiError ? err.message : "Something went wrong.");
  const load = useCallback(() => {
    hooksApi.list(token).then((r) => {
      setHooks(r.webhooks);
      setEvents(r.events);
    }, fail);
  }, [token]);
  useEffect(load, [load]);

  async function add() {
    setBusy(true);
    setError(null);
    try {
      const r = await hooksApi.create(token, url, picked);
      setSecret(r.secret);
      setUrl("");
      load();
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  }

  async function test(h: Webhook) {
    setTested((t) => ({ ...t, [h.id]: "Sending…" }));
    try {
      const r = await hooksApi.test(token, h.id);
      setTested((t) => ({ ...t, [h.id]: r.error ? `Failed: ${r.error}` : `Delivered (${r.status})` }));
      load();
    } catch (err) {
      setTested((t) => ({ ...t, [h.id]: err instanceof ApiError ? err.message : "Couldn't send." }));
    }
  }

  async function remove(h: Webhook) {
    if (!window.confirm(`Stop sending to ${h.url}?`)) return;
    await hooksApi.remove(token, h.id).then(load, fail);
  }

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="text-lg font-semibold">Webhooks</h2>
        <p className="text-sm text-muted">
          Outpost POSTs a signed JSON message to your own address when something finishes, for n8n, Home Assistant or a
          script. Check the <span className="font-mono">X-Outpost-Signature</span> header (HMAC-SHA256 of the body with the
          secret) to be sure it came from here.
        </p>
      </div>
      {hooks.length > 0 && (
        <ul className="flex flex-col divide-y divide-line rounded-2xl border border-line bg-surface">
          {hooks.map((h) => (
            <li key={h.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate font-mono text-sm">{h.url}</span>
                <span className="text-xs text-muted">
                  {h.events.map((e) => LABELS[e] ?? e).join(" · ")}
                  {h.last_at != null &&
                    ` · last sent ${ago(h.last_at)}: ${h.last_error ? `failed (${h.last_error})` : `OK (${h.last_status})`}`}
                </span>
                {tested[h.id] && <span className="text-xs text-soft">{tested[h.id]}</span>}
              </span>
              <Button variant="ghost" onClick={() => void test(h)}>
                Send a test
              </Button>
              <Button variant="danger" onClick={() => void remove(h)}>
                Remove
              </Button>
            </li>
          ))}
        </ul>
      )}
      {secret && (
        <div className="flex flex-col gap-2 rounded-2xl border border-ember bg-surface p-4" role="status">
          <span className="text-sm">
            <b>Copy this secret now</b>; it isn't shown again. Your receiver uses it to check the signature.
          </span>
          <code className="rounded-lg bg-deep px-3 py-2 font-mono text-sm break-all select-all">{secret}</code>
          <div className="flex gap-2">
            <Button variant="ghost" onClick={() => void navigator.clipboard.writeText(secret)}>
              Copy
            </Button>
            <Button variant="ghost" onClick={() => setSecret(null)}>
              Done
            </Button>
          </div>
        </div>
      )}
      <form
        className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5"
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <Field
          label="Send to"
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://192.168.1.30:5678/webhook/outpost"
        />
        <fieldset className="flex flex-wrap gap-4 text-sm">
          <legend className="mb-1.5 text-sm text-muted">When</legend>
          {events.map((ev) => (
            <label key={ev} className="flex items-center gap-2">
              <input
                type="checkbox"
                className="accent-ember"
                checked={picked.includes(ev)}
                onChange={(e) => setPicked((p) => (e.target.checked ? [...p, ev] : p.filter((x) => x !== ev)))}
              />
              {LABELS[ev] ?? ev}
            </label>
          ))}
        </fieldset>
        <ErrorNote>{error}</ErrorNote>
        <Button type="submit" className="self-start" disabled={busy || !url.trim() || picked.length === 0}>
          {busy ? "Adding…" : "Add webhook"}
        </Button>
      </form>
    </div>
  );
}

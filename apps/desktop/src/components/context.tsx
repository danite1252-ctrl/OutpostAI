import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, ApiError, type Memory, type Persona, type Session } from "../api";
import { Avatar, Icon, icons } from "./ui";

/** An app counts as connected if it used its key this recently. */
const ACTIVE_SECS = 10 * 60;

export function ago(unixSecs: number, now = Date.now() / 1000) {
  const s = Math.max(0, now - unixSecs);
  if (s < 90) return "just now";
  if (s < 3600) return `${Math.round(s / 60)} min ago`;
  if (s < 86400) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} days ago`;
}

/**
 * The chat's right-hand panel: who you're talking to, what it remembers, which apps
 * use your account, and how to reach this persona from them.
 */
export function ContextPanel({
  token,
  persona,
  refreshKey,
  onClose,
}: {
  token: string;
  persona: Persona | null;
  /** Changes when a reply finishes, so newly learned memories show. */
  refreshKey: number;
  onClose: () => void;
}) {
  const [memories, setMemories] = useState<Memory[]>([]);
  const [apps, setApps] = useState<Session[]>([]);
  const [text, setText] = useState("");
  const [onlyThis, setOnlyThis] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [showPrompt, setShowPrompt] = useState(false);

  const load = useCallback(() => {
    api.memory(token).then(setMemories, () => undefined);
    api.keys(token).then(setApps, () => undefined);
  }, [token]);

  useEffect(() => {
    load();
  }, [load, refreshKey]);

  // Connected-app dots go grey on their own.
  const [now, setNow] = useState(() => Date.now() / 1000);
  useEffect(() => {
    const t = window.setInterval(() => {
      setNow(Date.now() / 1000);
      api.keys(token).then(setApps, () => undefined);
    }, 60_000);
    return () => window.clearInterval(t);
  }, [token]);

  const mine = memories.filter((m) => !m.persona_id || m.persona_id === persona?.id);
  const manual = mine.filter((m) => m.source === "manual");
  const learned = mine.filter((m) => m.source === "auto");

  async function remember(e: FormEvent) {
    e.preventDefault();
    const t = text.trim();
    if (!t) return;
    setError(null);
    try {
      const m = await api.addMemory(token, t, onlyThis && persona ? persona.id : undefined);
      setMemories((list) => [m, ...list]);
      setText("");
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "That couldn't be saved.");
    }
  }

  async function forget(m: Memory) {
    try {
      await api.deleteMemory(token, m.id);
      setMemories((list) => list.filter((x) => x.id !== m.id));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "That couldn't be removed.");
    }
  }

  const model = persona ? `persona:${persona.slug}` : null;
  async function copy() {
    if (!model) return;
    try {
      await navigator.clipboard.writeText(model);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard not allowed: the text is still selectable.
    }
  }

  return (
    <aside aria-label="Context" className="flex w-80 shrink-0 flex-col gap-5 overflow-auto border-l border-line bg-surface p-5">
      <div className="flex items-center">
        <h2 className="flex-1 font-semibold">Context</h2>
        <button
          type="button"
          aria-label="Hide context"
          title="Hide"
          onClick={onClose}
          className="flex size-9 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-ink"
        >
          <Icon d={icons.close} size={16} />
        </button>
      </div>

      {persona ? (
        <div className="flex flex-col gap-2 rounded-2xl border border-line bg-night p-4">
          <div className="flex items-center gap-2.5">
            <Avatar name={persona.name} color={persona.avatar_color} size={28} />
            <b className="min-w-0 flex-1 truncate">{persona.name}</b>
          </div>
          {persona.system_prompt ? (
            <>
              <p className={`text-sm leading-relaxed whitespace-pre-wrap text-muted ${showPrompt ? "" : "line-clamp-4"}`}>
                {persona.system_prompt}
              </p>
              {persona.system_prompt.length > 180 && (
                <button type="button" onClick={() => setShowPrompt((s) => !s)} className="self-start text-xs text-ember hover:text-ember-hover">
                  {showPrompt ? "Show less" : "Show all"}
                </button>
              )}
            </>
          ) : (
            <p className="text-sm text-muted">No instructions: the model's own manner.</p>
          )}
          <p className="font-mono text-xs text-muted">
            {persona.model ?? "default model"}
            {persona.think ? " · thinking" : ""}
            {persona.temperature != null ? ` · temp ${persona.temperature}` : ""}
          </p>
        </div>
      ) : (
        <p className="text-sm text-muted">No persona on this chat.</p>
      )}

      <section className="flex flex-col gap-2" aria-labelledby="ctx-memory">
        <h3 id="ctx-memory" className="text-sm text-muted">
          Remembered
        </h3>
        {persona && !persona.use_memory ? (
          <p className="text-sm text-muted">Memory is off for {persona.name}. Turn it on in Personas.</p>
        ) : (
          <>
            {manual.length === 0 && learned.length === 0 && (
              <p className="text-sm text-muted">Nothing yet. Add a fact below, or let Outpost learn from chats (Personas › Memory).</p>
            )}
            <ul className="flex flex-col gap-1.5">
              {[...manual, ...learned].map((m) => (
                <li key={m.id} className="group flex items-start gap-2 rounded-xl bg-raised px-3 py-2 text-sm">
                  <span className="min-w-0 flex-1">
                    {m.text}
                    <span className="mt-0.5 block text-[11px] text-muted">
                      {m.source === "auto" ? "Learned from a chat" : "You added this"}
                      {m.persona_id ? " · this persona only" : ""}
                    </span>
                  </span>
                  <button
                    type="button"
                    aria-label={`Forget: ${m.text}`}
                    title="Forget"
                    onClick={() => void forget(m)}
                    className="shrink-0 rounded-md px-1.5 text-muted opacity-60 group-hover:opacity-100 hover:text-danger focus:opacity-100"
                  >
                    ×
                  </button>
                </li>
              ))}
            </ul>
            <form onSubmit={remember} className="flex flex-col gap-2">
              <label htmlFor="ctx-remember" className="sr-only">
                Something to remember
              </label>
              <input
                id="ctx-remember"
                value={text}
                onChange={(e) => setText(e.target.value)}
                placeholder="Remember something…"
                maxLength={1000}
                className="min-h-10 rounded-xl border border-line bg-deep px-3 text-sm text-ink focus:border-ember focus:outline-none"
              />
              {persona && (
                <label className="flex items-center gap-2 text-xs text-muted">
                  <input type="checkbox" checked={onlyThis} onChange={(e) => setOnlyThis(e.target.checked)} className="accent-ember" />
                  Only for {persona.name}
                </label>
              )}
            </form>
          </>
        )}
        {error && <p className="text-xs text-danger">{error}</p>}
      </section>

      <section className="flex flex-col gap-2" aria-labelledby="ctx-apps">
        <h3 id="ctx-apps" className="text-sm text-muted">
          Connected apps
        </h3>
        {apps.length === 0 ? (
          <p className="text-sm text-muted">No app keys yet. Make one in Settings to connect Hermes or another app.</p>
        ) : (
          <ul className="flex flex-col gap-2">
            {apps.map((a) => {
              const live = now - a.last_used_at < ACTIVE_SECS;
              const used = a.last_used_at > a.created_at + 1;
              return (
                <li key={a.id} className="flex items-center gap-2.5 text-sm">
                  <span className={`size-2 shrink-0 rounded-full ${live && used ? "bg-ok" : "bg-line"}`} aria-hidden="true" />
                  <span className="min-w-0 flex-1 truncate">{a.label}</span>
                  <span className="shrink-0 text-xs text-muted">{used ? ago(a.last_used_at, now) : "never used"}</span>
                </li>
              );
            })}
          </ul>
        )}
      </section>

      {model && (
        <section className="flex flex-col gap-2" aria-labelledby="ctx-api">
          <h3 id="ctx-api" className="text-sm text-muted">
            From other apps
          </h3>
          <div className="rounded-xl border border-line bg-night p-3 font-mono text-xs leading-relaxed text-muted">
            POST /v1/chat/completions
            <br />
            model: <span className="text-ink select-all">"{model}"</span>
          </div>
          <button type="button" onClick={() => void copy()} className="self-start text-xs text-ember hover:text-ember-hover">
            {copied ? "Copied" : "Copy model name"}
          </button>
        </section>
      )}
    </aside>
  );
}

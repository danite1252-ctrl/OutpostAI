// The activity log: every tool call a persona made, newest first. Who it was for, from which
// chat or routine, the action exactly as the model sent it, and what came of it. Owners see
// everyone's; people see their own.

import { useCallback, useEffect, useMemo, useState } from "react";
import { activityApi, ApiError, type ActivityEntry } from "../api";
import { Button, ErrorNote } from "../components/ui";

function message(e: unknown) {
  return e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
}

const OUTCOME: Record<ActivityEntry["outcome"], { label: string; tone: string }> = {
  done: { label: "Done", tone: "text-ok border-ok/40" },
  failed: { label: "Failed", tone: "text-danger border-danger/40" },
  not_allowed: { label: "Not allowed", tone: "text-danger border-danger/40" },
  skipped: { label: "Skipped", tone: "text-amber border-amber/40" },
  needs_ok: { label: "Needed an OK", tone: "text-amber border-amber/40" },
};

function when(at: number): string {
  const d = new Date(at * 1000);
  const today = new Date();
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (d.toDateString() === today.toDateString()) return time;
  return `${d.toLocaleDateString(undefined, { weekday: "short", day: "numeric", month: "short" })} ${time}`;
}

/** "name: kitchen lamp · domain: light": the arguments, readable. */
function argsWords(json: string): string {
  try {
    const v = JSON.parse(json) as unknown;
    if (!v || typeof v !== "object" || Array.isArray(v)) return json;
    const parts = Object.entries(v as Record<string, unknown>).map(
      ([k, x]) => `${k}: ${Array.isArray(x) ? x.join(", ") : typeof x === "object" && x ? JSON.stringify(x) : String(x)}`,
    );
    return parts.length ? parts.join(" · ") : "(nothing)";
  } catch {
    return json;
  }
}

export default function Activity({ token, isOwner, onOpenChat }: { token: string; isOwner: boolean; onOpenChat: (id: string) => void }) {
  const [entries, setEntries] = useState<ActivityEntry[] | null>(null);
  const [more, setMore] = useState(true);
  const [person, setPerson] = useState("");
  const [server, setServer] = useState("");
  const [error, setError] = useState<string | null>(null);
  // Everyone and every tool seen so far, for the filters.
  const [seen, setSeen] = useState<{ people: Map<string, string>; servers: Set<string> }>({ people: new Map(), servers: new Set() });

  const load = useCallback(
    async (before?: number) => {
      setError(null);
      try {
        const page = await activityApi.list(token, { person: person || undefined, server: server || undefined, before });
        setEntries((list) => (before ? [...(list ?? []), ...page] : page));
        setMore(page.length === 50);
        setSeen((s) => {
          const people = new Map(s.people);
          const servers = new Set(s.servers);
          for (const e of page) {
            if (e.person) people.set(e.account_id, e.person);
            servers.add(e.server);
          }
          return { people, servers };
        });
      } catch (e) {
        setError(message(e));
      }
    },
    [token, person, server],
  );
  useEffect(() => {
    setEntries(null);
    void load();
  }, [load]);

  const people = useMemo(() => [...seen.people.entries()].sort((a, b) => a[1].localeCompare(b[1])), [seen]);
  const servers = useMemo(() => [...seen.servers].sort(), [seen]);
  const select = "min-h-9 rounded-lg border border-line bg-surface px-2 text-sm text-ink";

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-6">
      <header className="flex flex-wrap items-end gap-3">
        <div className="flex-1">
          <h1 className="text-2xl font-semibold">Activity</h1>
          <p className="text-sm text-muted">
            Every tool call a persona made{isOwner ? ", for everyone" : " for you"}: the action exactly as the model sent it, and
            what came of it. Kept for 90 days.
          </p>
        </div>
        {isOwner && (
          <select aria-label="Whose" className={select} value={person} onChange={(e) => setPerson(e.target.value)}>
            <option value="">Everyone</option>
            {people.map(([id, name]) => (
              <option key={id} value={id}>
                {name}
              </option>
            ))}
          </select>
        )}
        <select aria-label="Which tool" className={select} value={server} onChange={(e) => setServer(e.target.value)}>
          <option value="">Every tool</option>
          {servers.map((s) => (
            <option key={s} value={s}>
              {s}
            </option>
          ))}
        </select>
        <Button variant="ghost" onClick={() => void load()}>
          Refresh
        </Button>
      </header>
      <ErrorNote>{error}</ErrorNote>
      {entries === null ? (
        <p className="text-sm text-muted">Loading…</p>
      ) : entries.length === 0 ? (
        <p className="text-sm text-muted">No tool calls yet. When a persona uses a tool (Home Assistant, Google, search…), it shows here.</p>
      ) : (
        <ul className="flex flex-col gap-2">
          {entries.map((e) => {
            const o = OUTCOME[e.outcome] ?? OUTCOME.done;
            return (
              <li key={e.id} className="rounded-xl border border-line bg-surface px-4 py-3">
                <div className="flex flex-wrap items-baseline gap-x-3 gap-y-1 text-sm">
                  <span className="w-28 shrink-0 text-muted">{when(e.at)}</span>
                  {isOwner && <span className="font-medium">{e.person ?? "Someone"}</span>}
                  <span className="font-mono">
                    {e.server} › {e.tool}
                  </span>
                  <span className={`rounded-full border px-2 text-xs ${o.tone}`}>{o.label}</span>
                  <span className="ml-auto text-xs text-muted">
                    {e.source === "routine" ? (
                      <>Routine "{e.routine_name ?? "deleted"}"</>
                    ) : e.source === "hands_free" ? (
                      "Hands-free"
                    ) : e.chat_id && e.chat_title !== null ? (
                      <button type="button" className="underline hover:text-ink" onClick={() => onOpenChat(e.chat_id!)}>
                        {e.chat_title}
                      </button>
                    ) : (
                      "A chat"
                    )}
                  </span>
                </div>
                <p className="mt-1 break-words font-mono text-xs text-soft">{argsWords(e.arguments)}</p>
                <details className="mt-1 text-xs">
                  <summary className="cursor-pointer text-muted">What came back</summary>
                  <p className="mt-1 whitespace-pre-wrap break-words text-soft">{e.result || "(nothing)"}</p>
                  <p className="mt-1 break-all font-mono text-muted">{e.arguments}</p>
                </details>
              </li>
            );
          })}
        </ul>
      )}
      {entries && entries.length > 0 && more && (
        <Button variant="ghost" onClick={() => void load(entries[entries.length - 1].at)}>
          Older
        </Button>
      )}
    </div>
  );
}

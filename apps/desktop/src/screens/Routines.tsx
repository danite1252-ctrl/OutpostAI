// Routines: a persona doing something on its own, on a schedule ("weekdays at 7:30, give me
// a morning brief"). Each run is a reply in the routine's own chat.

import { useCallback, useEffect, useState } from "react";
import { api, GATEWAY, routinesApi, type Notify, type Persona, type Routine, type RoutineRun, type Schedule } from "../api";
import { Avatar, Button, ErrorNote, Field, areYouSure } from "../components/ui";

const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

const NOTIFY: { id: Notify; label: string; short: string }[] = [
  { id: "always", label: "Every time", short: "tells you every run" },
  { id: "worth", label: "Only when there's news", short: "tells you when there's news" },
  { id: "failures", label: "Only if it fails", short: "tells you if it fails" },
  { id: "never", label: "Never", short: "doesn't notify" },
];

function message(e: unknown) {
  return e instanceof Error ? e.message : "Something went wrong.";
}

/** "07:30" as this PC says times: "7:30 am". */
function clock(time: string) {
  const [h, m] = time.split(":").map(Number);
  const d = new Date();
  d.setHours(h, m, 0, 0);
  return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** A moment, as people say it: "today 7:30 am", "Mon 9 Nov, 7:30 am". */
export function when(unix: number) {
  const d = new Date(unix * 1000);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const today = new Date();
  const tomorrow = new Date(today.getTime() + 86400000);
  if (d.toDateString() === today.toDateString()) return `today ${time}`;
  if (d.toDateString() === tomorrow.toDateString()) return `tomorrow ${time}`;
  return `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}, ${time}`;
}

/** "Weekdays at 7:30 am", "Every 4 hours", "Once, Sat 7 Nov, 9:00 am". */
const EVENTS = [
  { id: "job_finished", label: "A picture, video or 3D model of yours finishes" },
  { id: "training_finished", label: "A character's training finishes" },
] as const;

export function describe(s: Schedule) {
  if (s.kind === "link") return "Starts from its link (or Run now)";
  if (s.kind === "event") return s.event === "training_finished" ? "When a training finishes" : "When a Create job finishes";
  if (s.kind === "every") return s.hours === 1 ? "Every hour" : `Every ${s.hours} hours`;
  if (s.kind === "once") return `Once, ${when(s.at)}`;
  const days = [...s.days].sort();
  const at = clock(s.time);
  if (days.length === 7) return `Every day at ${at}`;
  if (days.join() === "1,2,3,4,5") return `Weekdays at ${at}`;
  if (days.join() === "0,6") return `Weekends at ${at}`;
  return `${days.map((d) => DAY_NAMES[d]).join(", ")} at ${at}`;
}

export default function Routines({ token, onOpenChat }: { token: string; onOpenChat: (chatId: string) => void }) {
  const [routines, setRoutines] = useState<Routine[] | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [editing, setEditing] = useState<Routine | "new" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [r, p] = await Promise.all([routinesApi.list(token), api.personas(token)]);
      setRoutines(r);
      setPersonas(p);
    } catch (e) {
      setError(message(e));
    }
  }, [token]);
  useEffect(() => {
    void load();
    // Keep "running" and the last results fresh.
    const t = window.setInterval(() => void routinesApi.list(token).then(setRoutines, () => undefined), 10000);
    return () => window.clearInterval(t);
  }, [load, token]);

  const replace = (r: Routine) => setRoutines((list) => (list ?? []).map((x) => (x.id === r.id ? r : x)));
  async function act(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(message(e));
    }
  }

  return (
    <div className="mx-auto flex max-w-4xl flex-col gap-6">
      <header className="flex flex-wrap items-end gap-4">
        <div className="min-w-0 flex-1">
          <h1 className="text-2xl font-semibold">Routines</h1>
          <p className="mt-1 text-sm text-muted">
            A persona doing something on its own, on a schedule: a morning brief, checking for updates, a weekly summary. Each run
            is a reply in the routine's own chat.
          </p>
        </div>
        <Button onClick={() => setEditing("new")}>New routine</Button>
      </header>
      <ErrorNote>{error}</ErrorNote>

      {editing && (
        <RoutineEditor
          token={token}
          routine={editing === "new" ? null : editing}
          personas={personas}
          onSaved={(r) => {
            setRoutines((list) => (editing === "new" ? [...(list ?? []), r] : (list ?? []).map((x) => (x.id === r.id ? r : x))));
            setEditing(null);
          }}
          onCancel={() => setEditing(null)}
        />
      )}

      {routines === null ? (
        <p className="text-sm text-muted">Loading…</p>
      ) : routines.length === 0 && !editing ? (
        <div className="rounded-2xl border border-dashed border-line p-8 text-center">
          <p className="text-muted">No routines yet.</p>
          <p className="mt-1 text-sm text-muted">
            Try one like "Every weekday at 7:30: give me the weather and anything on my calendar today."
          </p>
        </div>
      ) : (
        <ul className="flex flex-col gap-3">
          {routines.map((r) => (
            <RoutineCard
              key={r.id}
              token={token}
              routine={r}
              persona={personas.find((p) => p.id === r.persona_id) ?? null}
              onEdit={() => setEditing(r)}
              onOpenChat={onOpenChat}
              onChanged={replace}
              onDeleted={() => setRoutines((list) => (list ?? []).filter((x) => x.id !== r.id))}
              act={act}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

function RoutineCard({
  token,
  routine: r,
  persona,
  onEdit,
  onOpenChat,
  onChanged,
  onDeleted,
  act,
}: {
  token: string;
  routine: Routine;
  persona: Persona | null;
  onEdit: () => void;
  onOpenChat: (chatId: string) => void;
  onChanged: (r: Routine) => void;
  onDeleted: () => void;
  act: (work: () => Promise<unknown>) => Promise<void>;
}) {
  const [runs, setRuns] = useState<RoutineRun[] | null>(null);
  const running = r.last_status === "running";
  return (
    <li className={`flex flex-col gap-3 rounded-2xl border bg-surface p-4 ${r.enabled ? "border-line" : "border-line/60 opacity-75"}`}>
      <div className="flex flex-wrap items-start gap-3">
        {persona ? <Avatar name={persona.name} color={persona.avatar_color} size={36} /> : <span className="size-9 rounded-full bg-raised" />}
        <div className="min-w-0 flex-1">
          <p className="font-semibold">{r.name}</p>
          <p className="text-sm text-muted">
            {describe(r.schedule)}
            {persona ? ` · ${persona.name}` : ""} · {NOTIFY.find((n) => n.id === r.notify)?.short}
          </p>
          <p className="mt-1 text-sm">
            {!r.enabled ? (
              <span className="text-muted">Paused</span>
            ) : running ? (
              <span className="text-amber">Running now…</span>
            ) : r.next_at ? (
              <span className="text-soft">Next: {when(r.next_at)}</span>
            ) : r.schedule.kind === "link" || r.schedule.kind === "event" ? (
              <span className="text-soft">Waiting</span>
            ) : (
              <span className="text-muted">Done (it ran once)</span>
            )}
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <Button variant="ghost" disabled={running} onClick={() => void act(async () => onChanged(await routinesApi.runNow(token, r.id)))}>
            Run now
          </Button>
          {r.chat_id && (
            <Button variant="ghost" onClick={() => onOpenChat(r.chat_id!)}>
              Open chat
            </Button>
          )}
          <Button variant="ghost" onClick={onEdit}>
            Edit
          </Button>
          <Button
            variant="ghost"
            onClick={() => void act(async () => onChanged(await routinesApi.update(token, r.id, { enabled: !r.enabled })))}
          >
            {r.enabled ? "Pause" : "Resume"}
          </Button>
          <Button
            variant="danger"
            onClick={() =>
              void act(async () => {
                if (!(await areYouSure(`Delete the routine "${r.name}"? Its chat stays.`))) return;
                await routinesApi.remove(token, r.id);
                onDeleted();
              })
            }
          >
            Delete
          </Button>
        </div>
      </div>
      {r.last_at && r.last_status !== "running" && (
        <div className={`rounded-xl px-3 py-2 text-sm ${r.last_status === "failed" ? "bg-danger/10 text-danger" : "bg-raised text-soft"}`}>
          <span className="font-medium">
            {r.last_status === "failed" ? "Last run failed" : "Last run"} ({when(r.last_at)}):
          </span>{" "}
          {r.last_note ?? ""}
        </div>
      )}
      <LinkPanel token={token} routine={r} onChanged={onChanged} act={act} />
      <details
        onToggle={(e) => {
          if ((e.target as HTMLDetailsElement).open) void routinesApi.runs(token, r.id).then(setRuns, () => setRuns([]));
        }}
      >
        <summary className="cursor-pointer text-xs text-muted">Earlier runs</summary>
        {runs === null ? (
          <p className="mt-2 text-xs text-muted">Loading…</p>
        ) : runs.length === 0 ? (
          <p className="mt-2 text-xs text-muted">It hasn't run yet.</p>
        ) : (
          <ul className="mt-2 flex flex-col gap-1 text-xs">
            {runs.map((run) => (
              <li key={run.id} className="flex gap-2">
                <span className={run.status === "failed" ? "text-danger" : run.status === "running" ? "text-amber" : "text-ok"}>
                  {run.status === "ok" ? "Done" : run.status === "failed" ? "Failed" : "Running"}
                </span>
                <span className="text-muted">
                  {when(run.started_at)}
                  {run.reason === "now" ? " (Run now)" : run.reason === "link" ? " (from its link)" : run.reason === "event" ? " (something happened)" : ""}
                </span>
                <span className="min-w-0 flex-1 truncate text-soft">{run.note ?? ""}</span>
              </li>
            ))}
          </ul>
        )}
      </details>
    </li>
  );
}

/** Its secret link: for starting it from n8n, Home Assistant or anything that can call a web address. */
function LinkPanel({
  token,
  routine: r,
  onChanged,
  act,
}: {
  token: string;
  routine: Routine;
  onChanged: (r: Routine) => void;
  act: (work: () => Promise<unknown>) => Promise<void>;
}) {
  const [shown, setShown] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const url = shown ? `${GATEWAY}/api/hooks/routines/${shown}` : null;
  const local = GATEWAY.includes("127.0.0.1") || GATEWAY.includes("localhost");
  async function make(on: boolean) {
    await act(async () => {
      const made = await routinesApi.setLink(token, r.id, on);
      onChanged(made.routine);
      setShown(made.token);
      setCopied(false);
    });
  }
  if (!r.has_link && r.schedule.kind !== "link" && !url)
    return (
      <button type="button" className="self-start text-xs text-muted underline hover:text-ink" onClick={() => void make(true)}>
        Also start it from a secret link…
      </button>
    );
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-line px-3 py-2 text-sm">
      {!r.has_link ? (
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-muted">It starts from a secret link, which it hasn't got yet.</span>
          <Button variant="ghost" onClick={() => void make(true)}>
            Make its link
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-3">
          <span className="text-soft">Has a secret link</span>
          <Button variant="ghost" onClick={() => void make(true)}>
            Make a new link
          </Button>
          <Button
            variant="danger"
            onClick={() =>
              void (async () => {
                if (await areYouSure("Remove its link? Anything using it stops working.")) await make(false);
              })()
            }
          >
            Remove link
          </Button>
        </div>
      )}
      {url && (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-amber">Copy it now: it's only shown this once. Anyone with it can start this routine.</p>
          <div className="flex items-center gap-2">
            <code className="min-w-0 flex-1 truncate rounded-lg bg-deep px-2 py-1.5 text-xs">{url}</code>
            <Button
              variant="ghost"
              onClick={() =>
                void navigator.clipboard.writeText(url).then(
                  () => setCopied(true),
                  () => setCopied(false),
                )
              }
            >
              {copied ? "Copied" : "Copy"}
            </Button>
          </div>
          <p className="text-xs text-muted">
            Call it with POST (or GET). Anything sent with it (text, or JSON like {'{"who": "Sam"}'}) is passed to the routine as
            information. Outpost's certificate is its own, so in n8n turn on "Ignore SSL issues", and in Home Assistant use
            verify_ssl: false.
            {local ? " From another machine, use this PC's network address instead of 127.0.0.1." : ""}
          </p>
        </div>
      )}
    </div>
  );
}

type Kind = "daily" | "weekdays" | "days" | "every" | "once" | "link" | "event";

function kindOf(s: Schedule | null): Kind {
  if (!s) return "weekdays";
  if (s.kind === "every") return "every";
  if (s.kind === "once") return "once";
  if (s.kind === "link") return "link";
  if (s.kind === "event") return "event";
  const d = [...s.days].sort().join();
  if (d === "0,1,2,3,4,5,6") return "daily";
  if (d === "1,2,3,4,5") return "weekdays";
  return "days";
}

/** A local date and time for `<input type="datetime-local">`. */
function localInput(unix: number) {
  const d = new Date(unix * 1000);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

function RoutineEditor({
  token,
  routine,
  personas,
  onSaved,
  onCancel,
}: {
  token: string;
  routine: Routine | null;
  personas: Persona[];
  onSaved: (r: Routine) => void;
  onCancel: () => void;
}) {
  const s = routine?.schedule ?? null;
  const [name, setName] = useState(routine?.name ?? "");
  const [personaId, setPersonaId] = useState(routine?.persona_id ?? personas[0]?.id ?? "");
  const [instructions, setInstructions] = useState(routine?.instructions ?? "");
  const [kind, setKind] = useState<Kind>(kindOf(s));
  const [time, setTime] = useState(s?.kind === "days" ? s.time : "07:30");
  const [days, setDays] = useState<number[]>(s?.kind === "days" ? s.days : [1, 2, 3, 4, 5]);
  const [hours, setHours] = useState(s?.kind === "every" ? s.hours : 4);
  const [at, setAt] = useState(localInput(s?.kind === "once" ? s.at : Math.floor(Date.now() / 1000) + 3600));
  const [notify, setNotify] = useState<Notify>(routine?.notify ?? "always");
  const [event, setEvent] = useState<"job_finished" | "training_finished">(s?.kind === "event" ? s.event : "job_finished");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function schedule(): Schedule {
    switch (kind) {
      case "daily":
        return { kind: "days", time, days: [0, 1, 2, 3, 4, 5, 6] };
      case "weekdays":
        return { kind: "days", time, days: [1, 2, 3, 4, 5] };
      case "days":
        return { kind: "days", time, days };
      case "every":
        return { kind: "every", hours };
      case "once":
        return { kind: "once", at: Math.floor(new Date(at).getTime() / 1000) };
      case "link":
        return { kind: "link" };
      case "event":
        return { kind: "event", event };
    }
  }

  async function save() {
    setBusy(true);
    setError(null);
    try {
      const input = { name: name.trim(), persona_id: personaId, instructions: instructions.trim(), schedule: schedule(), notify };
      onSaved(routine ? await routinesApi.update(token, routine.id, input) : await routinesApi.create(token, input));
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const KINDS: { id: Kind; label: string }[] = [
    { id: "daily", label: "Every day" },
    { id: "weekdays", label: "Weekdays" },
    { id: "days", label: "Chosen days" },
    { id: "every", label: "Every few hours" },
    { id: "once", label: "Once" },
    { id: "link", label: "From a link" },
    { id: "event", label: "When something happens" },
  ];

  return (
    <section className="flex flex-col gap-4 rounded-2xl border border-ember/50 bg-surface p-5" aria-label={routine ? "Edit routine" : "New routine"}>
      <h2 className="text-lg font-semibold">{routine ? `Edit "${routine.name}"` : "New routine"}</h2>
      <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Morning brief" maxLength={80} />
      <label className="flex flex-col gap-1.5 text-sm text-muted">
        Persona (its instructions, model and tools do the work)
        <select
          className="min-h-11 rounded-xl border border-line bg-surface px-3 text-base text-ink focus:border-ember focus:outline-none"
          value={personaId}
          onChange={(e) => setPersonaId(e.target.value)}
        >
          <option value="">No persona (the default model)</option>
          {personas.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name}
            </option>
          ))}
        </select>
      </label>
      <label className="flex flex-col gap-1.5 text-sm text-muted">
        What it should do
        <textarea
          className="min-h-28 rounded-xl border border-line bg-surface px-3.5 py-2.5 text-base text-ink placeholder:text-muted/60 focus:border-ember focus:outline-none"
          value={instructions}
          onChange={(e) => setInstructions(e.target.value)}
          placeholder="Give me today's weather for Sydney and anything on my calendar, in a few lines."
          maxLength={4000}
        />
      </label>

      <fieldset className="flex flex-col gap-3">
        <legend className="mb-2 text-sm text-muted">When it runs</legend>
        <div className="flex flex-wrap gap-2" role="radiogroup">
          {KINDS.map((k) => (
            <button
              key={k.id}
              type="button"
              role="radio"
              aria-checked={kind === k.id}
              onClick={() => setKind(k.id)}
              className={`min-h-10 rounded-full border px-4 text-sm ${kind === k.id ? "border-ember bg-ember/10 text-ink" : "border-line text-soft hover:bg-raised"}`}
            >
              {k.label}
            </button>
          ))}
        </div>
        {kind === "days" && (
          <div className="flex flex-wrap gap-2" aria-label="Days">
            {DAY_NAMES.map((d, i) => (
              <label key={d} className="flex min-h-10 cursor-pointer items-center gap-2 rounded-full border border-line px-3 text-sm">
                <input
                  type="checkbox"
                  checked={days.includes(i)}
                  onChange={(e) => setDays((ds) => (e.target.checked ? [...ds, i].sort() : ds.filter((x) => x !== i)))}
                />
                {d}
              </label>
            ))}
          </div>
        )}
        {(kind === "daily" || kind === "weekdays" || kind === "days") && (
          <Field label="At" type="time" value={time} onChange={(e) => setTime(e.target.value)} />
        )}
        {kind === "every" && (
          <Field
            label="Every how many hours"
            type="number"
            min={1}
            max={168}
            value={hours}
            onChange={(e) => setHours(Math.max(1, Math.min(168, Number(e.target.value) || 1)))}
            hint="It first runs that long after you save it."
          />
        )}
        {kind === "once" && <Field label="On" type="datetime-local" value={at} onChange={(e) => setAt(e.target.value)} />}
        {kind === "link" && (
          <p className="text-sm text-muted">
            It runs when something calls its secret link (n8n, Home Assistant, a script), or when you press Run now. Make the link
            once it's saved.
          </p>
        )}
        {kind === "event" && (
          <div className="flex flex-col gap-2" role="radiogroup" aria-label="What starts it">
            {EVENTS.map((ev) => (
              <label key={ev.id} className="flex cursor-pointer items-center gap-2 text-sm">
                <input type="radio" name="event" checked={event === ev.id} onChange={() => setEvent(ev.id)} />
                {ev.label}
              </label>
            ))}
            <p className="text-xs text-muted">What finished (its name and how it went) is passed to the routine.</p>
          </div>
        )}
        <p className="text-xs text-muted">Times are this PC's time zone, daylight saving included, wherever the server is.</p>
      </fieldset>

      <fieldset className="flex flex-col gap-2">
        <legend className="mb-2 text-sm text-muted">Tell me (a notification on your phone and PC)</legend>
        <div className="flex flex-wrap gap-2" role="radiogroup">
          {NOTIFY.map((n) => (
            <button
              key={n.id}
              type="button"
              role="radio"
              aria-checked={notify === n.id}
              onClick={() => setNotify(n.id)}
              className={`min-h-10 rounded-full border px-4 text-sm ${notify === n.id ? "border-ember bg-ember/10 text-ink" : "border-line text-soft hover:bg-raised"}`}
            >
              {n.label}
            </button>
          ))}
        </div>
        {notify === "worth" && (
          <p className="text-xs text-muted">
            The persona decides: when there's nothing new it says so quietly in the chat, and you aren't notified. Failures are
            always told.
          </p>
        )}
      </fieldset>

      <ErrorNote>{error}</ErrorNote>
      <div className="flex gap-2">
        <Button disabled={busy || !name.trim() || !instructions.trim()} onClick={() => void save()}>
          {busy ? "Saving…" : routine ? "Save" : "Make the routine"}
        </Button>
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </section>
  );
}

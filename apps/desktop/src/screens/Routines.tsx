// Routines: a persona doing something on its own, on a schedule ("weekdays at 7:30, give me
// a morning brief"). Each run is a reply in the routine's own chat.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { api, GATEWAY, SERVER, routinesApi, type HomeAssistantThings, type Notify, type Persona, type QuietHours, type Routine, type RoutineAllow, type RoutineRun, type Schedule } from "../api";
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

/** The list's sections, each folding away. */
const GROUPS: { id: string; title: string; has: (s: Schedule) => boolean }[] = [
  { id: "schedule", title: "On a schedule", has: (s) => s.kind === "days" || s.kind === "every" },
  { id: "once", title: "Reminders and one-offs", has: (s) => s.kind === "once" },
  { id: "triggers", title: "From a link or when something happens", has: (s) => s.kind === "link" || s.kind === "event" },
];

function folded(id: string): boolean {
  try {
    return localStorage.getItem(`routines.folded.${id}`) === "1";
  } catch {
    return false;
  }
}

/** A section of the list that folds away (remembered on this PC). */
function Group({ id, title, count, children }: { id: string; title: string; count: number; children: ReactNode }) {
  const [shut, setShut] = useState(() => folded(id));
  function toggle() {
    setShut((v) => {
      try {
        localStorage.setItem(`routines.folded.${id}`, v ? "0" : "1");
      } catch {
        /* not kept: fine */
      }
      return !v;
    });
  }
  return (
    <section className="flex flex-col gap-3">
      <button
        type="button"
        aria-expanded={!shut}
        onClick={toggle}
        className="flex min-h-12 items-center gap-3 rounded-2xl border border-line bg-surface px-4 text-left hover:bg-raised"
      >
        <span className="flex-1 font-medium">
          {title} <span className="text-sm text-muted">({count})</span>
        </span>
        <svg aria-hidden viewBox="0 0 20 20" className={`size-4 text-muted transition-transform ${shut ? "" : "rotate-180"}`}>
          <path d="M5 8l5 5 5-5" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round" />
        </svg>
      </button>
      {!shut && <ul className="flex flex-col gap-3">{children}</ul>}
    </section>
  );
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
      <QuietPanel token={token} act={act} />

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
        <div className="flex flex-col gap-3">
          {GROUPS.map((g) => {
            const mine = routines.filter((r) => g.has(r.schedule));
            if (mine.length === 0) return null;
            return (
              <Group key={g.id} id={g.id} title={g.title} count={mine.length}>
                {mine.map((r) => (
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
              </Group>
            );
          })}
        </div>
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
              <span className="text-amber">{r.last_note ? r.last_note : "Running now…"}</span>
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
  // Using a server: its own address (this app's connection to it only works on this PC).
  const base = SERVER ? `https://${SERVER.host}:${SERVER.port}` : GATEWAY;
  const url = shown ? `${base}/api/hooks/routines/${shown}` : null;
  const local = !SERVER && (GATEWAY.includes("127.0.0.1") || GATEWAY.includes("localhost"));
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
          <HomeAssistantCall url={url} routine={r} />
        </div>
      )}
    </div>
  );
}

/** The link, ready to paste into Home Assistant's configuration.yaml, and how to call it. */
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  return (
    <Button
      variant="ghost"
      onClick={() =>
        void navigator.clipboard.writeText(text).then(
          () => setCopied(true),
          () => setCopied(false),
        )
      }
    >
      {copied ? "Copied" : "Copy"}
    </Button>
  );
}

function HomeAssistantCall({ url, routine: r }: { url: string; routine: Routine }) {
  const slug = r.name.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "") || "routine";
  const payload = r.camera ? `{"image": "{{ image | default('') }}", "camera": "${r.camera}"}` : `{"image": "{{ image | default('') }}"}`;
  const yaml = [
    "rest_command:",
    `  outpost_${slug}:`,
    `    url: "${url}"`,
    "    method: POST",
    "    verify_ssl: false",
    '    content_type: "application/json"',
    `    payload: '${payload}'`,
  ].join("\n");
  // A Nest doorbell: its ring, with the clip preview's picture passed on.
  const nest = [
    `alias: Outpost - ${r.name}`,
    "triggers:",
    "  - trigger: event",
    "    event_type: nest_event",
    "    event_data:",
    "      type: doorbell_chime",
    "actions:",
    `  - action: rest_command.outpost_${slug}`,
    "    data:",
    `      image: "{{ trigger.event.data.attachment.image | default('') }}"`,
  ].join("\n");
  return (
    <details className="text-xs">
      <summary className="cursor-pointer text-muted">For Home Assistant (a doorbell, motion, a button…)</summary>
      <div className="mt-2 flex flex-col gap-2">
        <p className="text-muted">
          1. Add this to Home Assistant's configuration.yaml and restart Home Assistant:
        </p>
        <pre className="overflow-x-auto rounded-lg bg-deep px-2 py-1.5">{yaml}</pre>
        <CopyButton text={yaml} />
        <p className="text-muted">
          2. A Google Nest doorbell: make a new automation, choose ⋮ › Edit in YAML, and paste this. It runs when the bell is
          pressed, and passes on the picture of who pressed it (its clip preview). For another camera or a button, make an
          automation with that trigger and the action <code>rest_command.outpost_{slug}</code>.
        </p>
        <pre className="overflow-x-auto rounded-lg bg-deep px-2 py-1.5">{nest}</pre>
        <CopyButton text={nest} />
      </div>
    </details>
  );
}

/** Ready-made routines to start from (the person fills in the parts in <angle brackets>). */
const READY: { name: string; instructions: string; schedule: Schedule; notify: Notify; needs?: string }[] = [
  {
    name: "Morning brief",
    instructions: "Give me a short morning brief: today's weather where I live, and anything on my calendar today. A few lines, no more.",
    schedule: { kind: "days", time: "07:00", days: [1, 2, 3, 4, 5] },
    notify: "always",
    needs: "a web search tool, and Google for the calendar",
  },
  {
    name: "Update checker",
    instructions:
      "Open https://papermc.io/downloads/paper and find the newest Paper version and build number. Compare it with what you found last time in this chat, and say if there's a newer one (its version and build).",
    schedule: { kind: "every", hours: 6 },
    notify: "worth",
    needs: "a tool that reads web pages",
  },
  {
    name: "News on a topic",
    instructions: "Search the web for news from the last day about <your topic>. Give me the three most important items, a line each, with links.",
    schedule: { kind: "days", time: "08:00", days: [0, 1, 2, 3, 4, 5, 6] },
    notify: "worth",
    needs: "a web search tool",
  },
  {
    name: "Week ahead",
    instructions: "Look at my calendar for this week. Tell me what's coming up, and anything I should prepare for.",
    schedule: { kind: "days", time: "07:30", days: [1] },
    notify: "always",
    needs: "Google (calendar)",
  },
  {
    name: "Who's at the door",
    instructions:
      "Look at the doorbell's pictures and say in one short sentence who or what is at the door: a courier with a parcel, someone you don't know, a neighbour's cat, nobody. Describe, don't guess names.",
    schedule: { kind: "link" },
    notify: "always",
    needs: "Home Assistant, a camera, and a persona whose model can see",
  },
  {
    name: "Reminder",
    instructions: "Remind me to <what>.",
    schedule: { kind: "once", at: Math.floor(Date.now() / 1000 / 60) * 60 + 3600 },
    notify: "always",
  },
  {
    name: "Caption new creations",
    instructions: "Write a one-line caption for what was just made, and one idea to make it better.",
    schedule: { kind: "event", event: "job_finished" },
    notify: "always",
  },
];

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
  const [ready, setReady] = useState<string | null>(null);
  function startFrom(t: (typeof READY)[number]) {
    const sc = t.schedule;
    setReady(t.name);
    setName(t.name);
    setInstructions(t.instructions);
    setNotify(t.notify);
    setKind(kindOf(sc));
    if (sc.kind === "days") {
      setTime(sc.time);
      setDays(sc.days);
    }
    if (sc.kind === "every") setHours(sc.hours);
    if (sc.kind === "once") setAt(localInput(Math.floor(Date.now() / 1000 / 60) * 60 + 3600));
    if (sc.kind === "event") setEvent(sc.event);
  }
  const [allow, setAllow] = useState<RoutineAllow>(routine?.allow ?? { tools: true, changes: "wait", commands: true });
  const [sayOn, setSayOn] = useState<string[]>(routine?.say_on ?? []);
  const [camera, setCamera] = useState(routine?.camera ?? "");
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
      const input = { name: name.trim(), persona_id: personaId, instructions: instructions.trim(), schedule: schedule(), notify, allow, say_on: sayOn, camera };
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
      {!routine && (
        <div className="flex flex-col gap-2">
          <p className="text-sm text-muted">Start from a ready-made one, or write your own (you can also ask in any chat: "remind me at 5 to…").</p>
          <div className="flex flex-wrap gap-2">
            {READY.map((t) => (
              <button
                key={t.name}
                type="button"
                onClick={() => startFrom(t)}
                className={`min-h-9 rounded-full border px-3 text-sm ${ready === t.name ? "border-ember bg-ember/10 text-ink" : "border-line text-soft hover:bg-raised"}`}
              >
                {t.name}
              </button>
            ))}
          </div>
          {ready && READY.find((t) => t.name === ready)?.needs && (
            <p className="text-xs text-muted">
              Its persona needs {READY.find((t) => t.name === ready)!.needs} (in the persona's or its chat's tools). Change anything in &lt;angle brackets&gt;.
            </p>
          )}
        </div>
      )}
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

      <CameraPick token={token} camera={camera} onChange={setCamera} />

      {notify !== "never" && notify !== "failures" && (
        <SayOutLoud token={token} personaId={personaId} sayOn={sayOn} onChange={setSayOn} />
      )}

      <fieldset className="flex flex-col gap-2 text-sm">
        <legend className="mb-2 text-sm text-muted">While it runs on its own (nobody is there to say OK)</legend>
        <label className="flex cursor-pointer items-center gap-2">
          <input type="checkbox" checked={allow.tools} onChange={(e) => setAllow({ ...allow, tools: e.target.checked })} />
          Use its chat's tools (web search, Google, the tools on your PC)
        </label>
        <label className="flex cursor-pointer items-center gap-2">
          <input
            type="checkbox"
            checked={allow.changes === "apply"}
            onChange={(e) => setAllow({ ...allow, changes: e.target.checked ? "apply" : "wait" })}
          />
          Make file changes in its chat's folders straight away (otherwise they wait for you to apply them)
        </label>
        <label className="flex cursor-pointer items-center gap-2">
          <input type="checkbox" checked={allow.commands} onChange={(e) => setAllow({ ...allow, commands: e.target.checked })} />
          Run commands, but only ones you've allowed "Always" in that folder
        </label>
        <p className="text-xs text-muted">
          To let it run a command, run it once in the routine's chat and choose Always. If its model runs on a lent PC that's off
          or paused, it waits up to an hour and tries again.
        </p>
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

/** Home Assistant's speakers and cameras (null while looking). */
function useHomeAssistant(token: string) {
  const [found, setFound] = useState<HomeAssistantThings | null>(null);
  useEffect(() => {
    routinesApi
      .speakers(token)
      .then(setFound)
      .catch((e) => setFound({ speakers: [], cameras: [], home_assistant: false, problem: message(e) }));
  }, [token]);
  return found;
}

/** A Home Assistant camera whose picture the routine looks at each run. */
function CameraPick({ token, camera, onChange }: { token: string; camera: string; onChange: (c: string) => void }) {
  const found = useHomeAssistant(token);
  const cameras = found?.cameras ?? [];
  if (!found || (!found.home_assistant && !camera)) return null;
  return (
    <label className="flex flex-col gap-1 text-sm">
      <span className="text-muted">Look at a camera (each run, its picture now goes with the instructions)</span>
      <select
        className="min-h-10 rounded-lg border border-line bg-surface px-2 text-ink"
        value={camera}
        onChange={(e) => onChange(e.target.value)}
      >
        <option value="">No camera</option>
        {cameras.map((c) => (
          <option key={c.id} value={c.id}>
            {c.name}
          </option>
        ))}
        {camera && !cameras.some((c) => c.id === camera) && <option value={camera}>{camera} (not in Home Assistant now)</option>}
      </select>
      {camera && (
        <span className="text-xs text-muted">
          Its persona's model needs to see pictures (a vision model). A link can name another camera each time:{" "}
          <code>{'{"camera": "camera.back_door"}'}</code>.
        </span>
      )}
      {found.problem && <span className="text-xs text-danger">Home Assistant's cameras couldn't be listed: {found.problem}</span>}
    </label>
  );
}

/** Where a routine says its result out loud: the phone, and Home Assistant's speakers. */
function SayOutLoud({
  token,
  personaId,
  sayOn,
  onChange,
}: {
  token: string;
  personaId: string;
  sayOn: string[];
  onChange: (s: string[]) => void;
}) {
  const found = useHomeAssistant(token);
  const [trying, setTrying] = useState<string | null>(null);
  const [tried, setTried] = useState<{ id: string; error: string | null } | null>(null);
  const flip = (id: string, on: boolean) => onChange(on ? [...sayOn.filter((x) => x !== id), id] : sayOn.filter((x) => x !== id));
  async function tryIt(id: string) {
    setTrying(id);
    setTried(null);
    try {
      await routinesApi.trySpeaker(token, id, personaId);
      setTried({ id, error: null });
    } catch (e) {
      setTried({ id, error: message(e) });
    } finally {
      setTrying(null);
    }
  }
  // Speakers it was set to that Home Assistant doesn't list now (renamed, unplugged).
  const missing = sayOn.filter((id) => id !== "phone" && found && !found.speakers.some((s) => s.id === id));
  return (
    <fieldset className="flex flex-col gap-2 text-sm">
      <legend className="mb-2 text-sm text-muted">Say it out loud too</legend>
      <label className="flex cursor-pointer items-center gap-2">
        <input type="checkbox" checked={sayOn.includes("phone")} onChange={(e) => flip("phone", e.target.checked)} />
        On my phone
      </label>
      {found === null ? (
        <p className="text-xs text-muted">Looking for speakers…</p>
      ) : found.speakers.length > 0 ? (
        found.speakers.map((s) => (
          <div key={s.id} className="flex flex-wrap items-center gap-2">
            <label className="flex cursor-pointer items-center gap-2">
              <input type="checkbox" checked={sayOn.includes(s.id)} onChange={(e) => flip(s.id, e.target.checked)} />
              {s.name}
            </label>
            <button
              type="button"
              className="text-xs text-muted underline hover:text-ink disabled:opacity-50"
              disabled={trying !== null}
              onClick={() => void tryIt(s.id)}
            >
              {trying === s.id ? "Saying…" : "Try it"}
            </button>
            {tried?.id === s.id && (
              <span className={`text-xs ${tried.error ? "text-danger" : "text-ok"}`}>{tried.error ?? "Sent. Did you hear it?"}</span>
            )}
          </div>
        ))
      ) : (
        <p className="text-xs text-muted">
          {found.problem
            ? `Home Assistant's speakers couldn't be listed: ${found.problem}`
            : found.home_assistant
              ? "Home Assistant has no speakers."
              : "To use your speakers, add Home Assistant as a tool (Tools)."}
        </p>
      )}
      {missing.map((id) => (
        <label key={id} className="flex cursor-pointer items-center gap-2 text-muted">
          <input type="checkbox" checked onChange={() => flip(id, false)} />
          {id} (not in Home Assistant now)
        </label>
      ))}
      <p className="text-xs text-muted">
        When it tells you, it also says the result out loud, in the persona's voice when Outpost is Home Assistant's voice
        service. Not during quiet hours or tools-off hours.
      </p>
    </fieldset>
  );
}

/** Quiet hours: news from runs that finish in them is told when they end. */
function QuietPanel({ token, act }: { token: string; act: (work: () => Promise<unknown>) => Promise<void> }) {
  const [quiet, setQuiet] = useState<QuietHours | null | undefined>(undefined);
  const [from, setFrom] = useState("22:00");
  const [to, setTo] = useState("07:00");
  useEffect(() => {
    routinesApi.quiet(token).then(
      (q) => {
        setQuiet(q.quiet);
        if (q.quiet) {
          setFrom(q.quiet.from);
          setTo(q.quiet.to);
        }
      },
      () => setQuiet(null),
    );
  }, [token]);
  if (quiet === undefined) return null;
  const save = (q: QuietHours | null) => act(async () => setQuiet((await routinesApi.setQuiet(token, q)).quiet));
  const changed = quiet && (quiet.from !== from || quiet.to !== to);
  return (
    <div className="flex flex-wrap items-center gap-3 rounded-xl border border-line px-4 py-3 text-sm">
      <label className="flex cursor-pointer items-center gap-2">
        <input type="checkbox" checked={!!quiet} onChange={(e) => void save(e.target.checked ? { from, to } : null)} />
        Quiet hours
      </label>
      <input
        type="time"
        aria-label="Quiet from"
        className="min-h-9 rounded-lg border border-line bg-surface px-2 text-ink"
        value={from}
        onChange={(e) => setFrom(e.target.value)}
      />
      <span className="text-muted">to</span>
      <input
        type="time"
        aria-label="Quiet until"
        className="min-h-9 rounded-lg border border-line bg-surface px-2 text-ink"
        value={to}
        onChange={(e) => setTo(e.target.value)}
      />
      {changed && (
        <Button variant="ghost" onClick={() => void save({ from, to })}>
          Save
        </Button>
      )}
      <span className="basis-full text-xs text-muted">
        Routines still run, but you're only told when quiet hours end.
      </span>
    </div>
  );
}

// Routines on the phone: a persona doing something on its own, on a schedule. Make, change,
// pause and run them; see how each went, and open its chat for the full result.

import { useCallback, useEffect, useState } from "react";
import { Alert, Share, Text, View } from "react-native";
import { ListState, message, Screen } from "./admin";
import type { Outpost } from "./outpost";
import { Avatar, Button, C, Chip, ErrorText, Field, P, s } from "./ui";

type Notify = "always" | "worth" | "failures" | "never";
const NOTIFY: { id: Notify; label: string; short: string }[] = [
  { id: "always", label: "Every time", short: "tells you every run" },
  { id: "worth", label: "Only with news", short: "tells you when there's news" },
  { id: "failures", label: "Only if it fails", short: "tells you if it fails" },
  { id: "never", label: "Never", short: "doesn't notify" },
];

type Schedule =
  | { kind: "once"; at: number }
  | { kind: "days"; time: string; days: number[] }
  | { kind: "every"; hours: number }
  | { kind: "link" }
  | { kind: "event"; event: "job_finished" | "training_finished" };

const EVENTS = [
  { id: "job_finished", label: "A picture, video or 3D model finishes" },
  { id: "training_finished", label: "A training finishes" },
] as const;

interface Routine {
  id: string;
  name: string;
  persona_id: string | null;
  instructions: string;
  schedule: Schedule;
  enabled: boolean;
  notify: Notify;
  has_link: boolean;
  allow: Allow;
  chat_id: string | null;
  next_at: number | null;
  last_at: number | null;
  last_status: "ok" | "failed" | "running" | null;
  last_note: string | null;
}
/** What a routine may do while nobody's watching. */
interface Allow {
  tools: boolean;
  changes: "wait" | "apply";
  commands: boolean;
}
interface Quiet {
  from: string;
  to: string;
}
interface Persona {
  id: string;
  name: string;
  avatar_color: string;
}

const DAY_NAMES = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

function clock(time: string) {
  const [h, m] = time.split(":").map(Number);
  const d = new Date();
  d.setHours(h, m, 0, 0);
  return d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

function when(unix: number) {
  const d = new Date(unix * 1000);
  const time = d.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const today = new Date();
  if (d.toDateString() === today.toDateString()) return `today ${time}`;
  if (d.toDateString() === new Date(today.getTime() + 86400000).toDateString()) return `tomorrow ${time}`;
  return `${d.toLocaleDateString([], { weekday: "short", day: "numeric", month: "short" })}, ${time}`;
}

function describe(s: Schedule) {
  if (s.kind === "link") return "Starts from its link (or Run now)";
  if (s.kind === "event") return s.event === "training_finished" ? "When a training finishes" : "When a Create job finishes";
  if (s.kind === "every") return s.hours === 1 ? "Every hour" : `Every ${s.hours} hours`;
  if (s.kind === "once") return `Once, ${when(s.at)}`;
  const days = [...s.days].sort().join();
  const at = clock(s.time);
  if (days === "0,1,2,3,4,5,6") return `Every day at ${at}`;
  if (days === "1,2,3,4,5") return `Weekdays at ${at}`;
  if (days === "0,6") return `Weekends at ${at}`;
  return `${[...s.days].sort().map((d) => DAY_NAMES[d]).join(", ")} at ${at}`;
}

/** This phone's time zone, so a routine runs at your times. */
function myZone() {
  let zone: string | undefined;
  try {
    zone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  } catch {
    zone = undefined;
  }
  return { time_zone: zone, utc_offset_minutes: -new Date().getTimezoneOffset() };
}

export function RoutinesScreen({ api, onClose, onOpenChat }: { api: Outpost; onClose: () => void; onOpenChat: (chatId: string) => void }) {
  const [routines, setRoutines] = useState<Routine[] | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [editing, setEditing] = useState<Routine | "new" | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [r, p] = await Promise.all([api.get<Routine[]>("/api/routines"), api.get<Persona[]>("/api/personas")]);
      setRoutines(r);
      setPersonas(p);
    } catch (e) {
      setError(message(e));
    }
  }, [api]);
  useEffect(() => {
    void load();
    const t = setInterval(() => void api.get<Routine[]>("/api/routines").then(setRoutines, () => undefined), 10000);
    return () => clearInterval(t);
  }, [load, api]);

  const replace = (r: Routine) => setRoutines((list) => (list ?? []).map((x) => (x.id === r.id ? r : x)));
  async function act(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(message(e));
    }
  }

  if (editing)
    return (
      <RoutineEditor
        api={api}
        routine={editing === "new" ? null : editing}
        personas={personas}
        onSaved={(r) => {
          setRoutines((list) => (editing === "new" ? [...(list ?? []), r] : (list ?? []).map((x) => (x.id === r.id ? r : x))));
          setEditing(null);
        }}
        onClose={() => setEditing(null)}
      />
    );

  return (
    <Screen title="Routines" onClose={onClose}>
      <P muted>A persona doing something on its own, on a schedule. Each run is a reply in the routine's own chat.</P>
      <Button title="New routine" onPress={() => setEditing("new")} />
      <ErrorText>{error}</ErrorText>
      <QuietRow api={api} act={act} />
      <ListState
        loading={routines === null}
        error={null}
        empty={routines?.length === 0}
        emptyText='None yet. Try "Every weekday at 7:30: give me the weather and my calendar for today."'
      />
      {(routines ?? []).map((r) => {
        const persona = personas.find((p) => p.id === r.persona_id);
        const running = r.last_status === "running";
        return (
          <View key={r.id} style={[s.card, { gap: 8, opacity: r.enabled ? 1 : 0.7 }]}>
            <View style={s.row}>
              {persona && <Avatar name={persona.name} color={persona.avatar_color} size={32} />}
              <View style={{ flex: 1 }}>
                <Text style={{ color: C.ink, fontWeight: "600" }}>{r.name}</Text>
                <Text style={{ color: C.muted, fontSize: 13 }}>
                  {describe(r.schedule)}
                  {persona ? ` · ${persona.name}` : ""} · {NOTIFY.find((n) => n.id === r.notify)?.short}
                </Text>
                <Text style={{ color: !r.enabled ? C.muted : running ? C.amber : C.soft, fontSize: 13 }}>
                  {!r.enabled
                    ? "Paused"
                    : running
                      ? (r.last_note ?? "Running now…")
                      : r.next_at
                        ? `Next: ${when(r.next_at)}`
                        : r.schedule.kind === "link" || r.schedule.kind === "event"
                          ? "Waiting"
                          : "Done (it ran once)"}
                </Text>
              </View>
            </View>
            {r.last_at && !running && (
              <Text style={{ color: r.last_status === "failed" ? C.danger : C.soft, fontSize: 13 }} numberOfLines={4}>
                {r.last_status === "failed" ? "Last run failed" : "Last run"} ({when(r.last_at)}): {r.last_note ?? ""}
              </Text>
            )}
            <LinkRow api={api} routine={r} onChanged={replace} act={act} />
            <View style={[s.row, { flexWrap: "wrap" }]}>
              {!running && (
                <Chip label="Run now" onPress={() => void act(async () => replace(await api.json<Routine>("POST", `/api/routines/${r.id}/run`)))} />
              )}
              {r.chat_id && <Chip label="Open chat" onPress={() => onOpenChat(r.chat_id!)} />}
              <Chip label="Edit" onPress={() => setEditing(r)} />
              <Chip
                label={r.enabled ? "Pause" : "Resume"}
                onPress={() => void act(async () => replace(await api.json<Routine>("PUT", `/api/routines/${r.id}`, { enabled: !r.enabled })))}
              />
              <Chip
                label="Delete"
                color={C.danger}
                onPress={() =>
                  Alert.alert(`Delete "${r.name}"?`, "Its chat stays.", [
                    { text: "Keep", style: "cancel" },
                    {
                      text: "Delete",
                      style: "destructive",
                      onPress: () =>
                        void act(async () => {
                          await api.json("DELETE", `/api/routines/${r.id}`);
                          setRoutines((list) => (list ?? []).filter((x) => x.id !== r.id));
                        }),
                    },
                  ])
                }
              />
            </View>
          </View>
        );
      })}
    </Screen>
  );
}

/** Its secret link, for starting it from n8n, Home Assistant or anything that can call a web address. */
function LinkRow({
  api,
  routine: r,
  onChanged,
  act,
}: {
  api: Outpost;
  routine: Routine;
  onChanged: (r: Routine) => void;
  act: (work: () => Promise<unknown>) => Promise<void>;
}) {
  const [url, setUrl] = useState<string | null>(null);
  const make = (on: boolean) =>
    act(async () => {
      const made = await api.json<{ routine: Routine; token: string | null }>("PUT", `/api/routines/${r.id}/link`, { on });
      onChanged(made.routine);
      setUrl(made.token ? `https://${api.conn.hosts[0]}:${api.conn.port}/api/hooks/routines/${made.token}` : null);
    });
  if (!r.has_link && r.schedule.kind !== "link" && !url) return null;
  return (
    <View style={{ gap: 6 }}>
      <View style={[s.row, { flexWrap: "wrap", alignItems: "center" }]}>
        <Text style={{ color: C.muted, fontSize: 13, flex: 1 }}>{r.has_link ? "Has a secret link" : "Starts from a secret link (none yet)"}</Text>
        <Chip label={r.has_link ? "New link" : "Make its link"} onPress={() => void make(true)} />
        {r.has_link && (
          <Chip
            label="Remove link"
            color={C.danger}
            onPress={() =>
              Alert.alert("Remove its link?", "Anything using it stops working.", [
                { text: "Keep", style: "cancel" },
                { text: "Remove", style: "destructive", onPress: () => void make(false) },
              ])
            }
          />
        )}
      </View>
      {url && (
        <View style={{ gap: 4 }}>
          <Text style={{ color: C.amber, fontSize: 12 }}>Send it to yourself now: it's only shown this once. Anyone with it can start this routine.</Text>
          <Text selectable style={{ color: C.ink, fontSize: 12, fontFamily: "monospace" }}>
            {url}
          </Text>
          <Chip label="Share the link" onPress={() => void Share.share({ message: url })} />
        </View>
      )}
    </View>
  );
}

type Kind = "daily" | "weekdays" | "days" | "every" | "once" | "link" | "event";

function kindOf(s: Schedule | null): Kind {
  if (!s) return "weekdays";
  if (s.kind !== "days") return s.kind;
  const d = [...s.days].sort().join();
  return d === "0,1,2,3,4,5,6" ? "daily" : d === "1,2,3,4,5" ? "weekdays" : "days";
}

const pad = (n: number) => String(n).padStart(2, "0");

function RoutineEditor({
  api,
  routine,
  personas,
  onSaved,
  onClose,
}: {
  api: Outpost;
  routine: Routine | null;
  personas: Persona[];
  onSaved: (r: Routine) => void;
  onClose: () => void;
}) {
  const sch = routine?.schedule ?? null;
  const onceAt = new Date((sch?.kind === "once" ? sch.at : Math.floor(Date.now() / 1000) + 3600) * 1000);
  const [name, setName] = useState(routine?.name ?? "");
  const [personaId, setPersonaId] = useState(routine?.persona_id ?? personas[0]?.id ?? "");
  const [instructions, setInstructions] = useState(routine?.instructions ?? "");
  const [kind, setKind] = useState<Kind>(kindOf(sch));
  const [time, setTime] = useState(sch?.kind === "days" ? sch.time : "07:30");
  const [days, setDays] = useState<number[]>(sch?.kind === "days" ? sch.days : [1, 2, 3, 4, 5]);
  const [hours, setHours] = useState(String(sch?.kind === "every" ? sch.hours : 4));
  const [date, setDate] = useState(`${onceAt.getFullYear()}-${pad(onceAt.getMonth() + 1)}-${pad(onceAt.getDate())}`);
  const [onceTime, setOnceTime] = useState(`${pad(onceAt.getHours())}:${pad(onceAt.getMinutes())}`);
  const [notify, setNotify] = useState<Notify>(routine?.notify ?? "always");
  const [event, setEvent] = useState<"job_finished" | "training_finished">(sch?.kind === "event" ? sch.event : "job_finished");
  const [allow, setAllow] = useState<Allow>(routine?.allow ?? { tools: true, changes: "wait", commands: true });
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  function schedule(): Schedule | string {
    const okTime = (t: string) => /^([01]?\d|2[0-3]):[0-5]\d$/.test(t.trim());
    switch (kind) {
      case "link":
        return { kind: "link" };
      case "event":
        return { kind: "event", event };
      case "every": {
        const h = Number(hours);
        return Number.isInteger(h) && h >= 1 && h <= 168 ? { kind: "every", hours: h } : "Every 1 to 168 hours.";
      }
      case "once": {
        if (!/^\d{4}-\d{2}-\d{2}$/.test(date.trim()) || !okTime(onceTime)) return "Give the date like 2026-11-07 and the time like 09:00.";
        const [y, mo, d] = date.trim().split("-").map(Number);
        const [h, mi] = onceTime.trim().split(":").map(Number);
        return { kind: "once", at: Math.floor(new Date(y, mo - 1, d, h, mi).getTime() / 1000) };
      }
      default: {
        if (!okTime(time)) return "Give the time like 07:30 (24-hour).";
        const t = time.trim().padStart(5, "0");
        const chosen = kind === "daily" ? [0, 1, 2, 3, 4, 5, 6] : kind === "weekdays" ? [1, 2, 3, 4, 5] : days;
        return chosen.length ? { kind: "days", time: t, days: chosen } : "Pick at least one day.";
      }
    }
  }

  async function save() {
    const sc = schedule();
    if (typeof sc === "string") return setError(sc);
    setBusy(true);
    setError(null);
    try {
      const body = { name: name.trim(), persona_id: personaId, instructions: instructions.trim(), schedule: sc, notify, allow, ...myZone() };
      onSaved(routine ? await api.json<Routine>("PUT", `/api/routines/${routine.id}`, body) : await api.json<Routine>("POST", "/api/routines", body));
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
    <Screen title={routine ? `Edit "${routine.name}"` : "New routine"} onClose={onClose} back="‹ Routines">
      <Field label="Name" value={name} onChangeText={setName} placeholder="Morning brief" maxLength={80} />
      <Text style={s.label}>Persona (its instructions, model and tools do the work)</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        <Chip label="No persona" active={!personaId} onPress={() => setPersonaId("")} />
        {personas.map((p) => (
          <Chip key={p.id} label={p.name} active={personaId === p.id} onPress={() => setPersonaId(p.id)} />
        ))}
      </View>
      <Field
        label="What it should do"
        value={instructions}
        onChangeText={setInstructions}
        multiline
        style={{ minHeight: 110, textAlignVertical: "top" }}
        placeholder="Give me today's weather for Sydney and anything on my calendar, in a few lines."
        maxLength={4000}
      />
      <Text style={s.label}>When it runs</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {KINDS.map((k) => (
          <Chip key={k.id} label={k.label} active={kind === k.id} onPress={() => setKind(k.id)} />
        ))}
      </View>
      {kind === "days" && (
        <View style={[s.row, { flexWrap: "wrap" }]}>
          {DAY_NAMES.map((d, i) => (
            <Chip
              key={d}
              label={d}
              active={days.includes(i)}
              onPress={() => setDays((ds) => (ds.includes(i) ? ds.filter((x) => x !== i) : [...ds, i].sort()))}
            />
          ))}
        </View>
      )}
      {(kind === "daily" || kind === "weekdays" || kind === "days") && (
        <Field label="At (24-hour, like 07:30)" value={time} onChangeText={setTime} keyboardType="numbers-and-punctuation" />
      )}
      {kind === "link" && <P muted>It runs when something calls its secret link (n8n, Home Assistant), or Run now. Make the link once it's saved.</P>}
      {kind === "event" && (
        <View style={{ gap: 6 }}>
          {EVENTS.map((ev) => (
            <Chip key={ev.id} label={ev.label} active={event === ev.id} onPress={() => setEvent(ev.id)} />
          ))}
        </View>
      )}
      {kind === "every" && <Field label="Every how many hours" value={hours} onChangeText={setHours} keyboardType="number-pad" />}
      {kind === "once" && (
        <>
          <Field label="Date (like 2026-11-07)" value={date} onChangeText={setDate} keyboardType="numbers-and-punctuation" />
          <Field label="Time (24-hour, like 09:00)" value={onceTime} onChangeText={setOnceTime} keyboardType="numbers-and-punctuation" />
        </>
      )}
      <P muted>Times are this phone's time zone, daylight saving included.</P>
      <Text style={s.label}>Tell me (a notification on your phone and PC)</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {NOTIFY.map((n) => (
          <Chip key={n.id} label={n.label} active={notify === n.id} onPress={() => setNotify(n.id)} />
        ))}
      </View>
      {notify === "worth" && (
        <P muted>The persona decides: when there's nothing new it says so quietly in the chat. Failures are always told.</P>
      )}
      <Text style={s.label}>While it runs on its own (nobody is there to say OK)</Text>
      <View style={{ gap: 6 }}>
        <Chip label="Use its chat's tools (web search, your PC's tools)" active={allow.tools} onPress={() => setAllow({ ...allow, tools: !allow.tools })} />
        <Chip
          label="Make file changes straight away"
          active={allow.changes === "apply"}
          onPress={() => setAllow({ ...allow, changes: allow.changes === "apply" ? "wait" : "apply" })}
        />
        <Chip label={'Run commands allowed "Always" in the folder'} active={allow.commands} onPress={() => setAllow({ ...allow, commands: !allow.commands })} />
      </View>
      <P muted>
        File changes otherwise wait for you to apply them. If its model runs on a lent PC that's off or paused, it waits up to an hour and
        tries again.
      </P>
      <ErrorText>{error}</ErrorText>
      <Button
        title={routine ? "Save" : "Make the routine"}
        busy={busy}
        disabled={!name.trim() || !instructions.trim()}
        onPress={() => void save()}
      />
    </Screen>
  );
}

/** Quiet hours: news from runs that finish in them is told when they end. */
function QuietRow({ api, act }: { api: Outpost; act: (work: () => Promise<unknown>) => Promise<void> }) {
  const [quiet, setQuiet] = useState<Quiet | null | undefined>(undefined);
  const [from, setFrom] = useState("22:00");
  const [to, setTo] = useState("07:00");
  useEffect(() => {
    api.get<{ quiet: Quiet | null }>("/api/routines/quiet").then(
      (q) => {
        setQuiet(q.quiet);
        if (q.quiet) {
          setFrom(q.quiet.from);
          setTo(q.quiet.to);
        }
      },
      () => setQuiet(null),
    );
  }, [api]);
  if (quiet === undefined) return null;
  const save = (q: Quiet | null) =>
    act(async () => setQuiet((await api.json<{ quiet: Quiet | null }>("PUT", "/api/routines/quiet", { quiet: q })).quiet));
  const changed = !!quiet && (quiet.from !== from.trim() || quiet.to !== to.trim());
  return (
    <View style={[s.card, { gap: 8 }]}>
      <Chip label={quiet ? `Quiet hours on (${quiet.from} to ${quiet.to})` : "Quiet hours off"} active={!!quiet} onPress={() => void save(quiet ? null : { from: from.trim(), to: to.trim() })} />
      <View style={s.row}>
        <View style={{ flex: 1 }}>
          <Field label="From" value={from} onChangeText={setFrom} keyboardType="numbers-and-punctuation" />
        </View>
        <View style={{ flex: 1 }}>
          <Field label="To" value={to} onChangeText={setTo} keyboardType="numbers-and-punctuation" />
        </View>
      </View>
      {changed && <Button title="Save quiet hours" onPress={() => void save({ from: from.trim(), to: to.trim() })} />}
      <P muted>Routines still run, but you're only told when quiet hours end.</P>
    </View>
  );
}

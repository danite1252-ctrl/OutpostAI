// Routines on the phone: a persona doing something on its own, on a schedule. Make, change,
// pause and run them; see how each went, and open its chat for the full result.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Alert, Pressable, Share, Text, View } from "react-native";
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

/** Ready-made routines to start from (the person fills in the parts in <angle brackets>). */
const READY: { name: string; instructions: string; schedule: Schedule; notify: Notify; needs?: string; quietSkip?: boolean }[] = [
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
    quietSkip: true,
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
  /** Where it says its result out loud: "phone", and Home Assistant speakers. */
  say_on: string[];
  /** A Home Assistant camera whose picture it looks at each run. */
  camera: string | null;
  /** Name people in Known faces it sees. */
  recognise: boolean;
  /** In quiet hours, don't tell at all (rather than when they end). */
  quiet_skip: boolean;
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

/** The list's sections, each folding away. */
const GROUPS: { id: string; title: string; has: (s: Schedule) => boolean }[] = [
  { id: "schedule", title: "On a schedule", has: (s) => s.kind === "days" || s.kind === "every" },
  { id: "once", title: "Reminders and one-offs", has: (s) => s.kind === "once" },
  { id: "triggers", title: "From a link or an event", has: (s) => s.kind === "link" || s.kind === "event" },
];

function Group({ title, count, children }: { title: string; count: number; children: ReactNode }) {
  const [shut, setShut] = useState(false);
  return (
    <View style={{ gap: 10 }}>
      <Pressable
        accessibilityRole="button"
        accessibilityState={{ expanded: !shut }}
        onPress={() => setShut((v) => !v)}
        style={[s.card, { flexDirection: "row", alignItems: "center", minHeight: 48 }]}
      >
        <Text style={{ color: C.ink, fontWeight: "600", flex: 1 }}>
          {title} <Text style={{ color: C.muted, fontWeight: "400" }}>({count})</Text>
        </Text>
        <Text style={{ color: C.muted, fontSize: 16 }}>{shut ? "⌄" : "⌃"}</Text>
      </Pressable>
      {!shut && children}
    </View>
  );
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
      {GROUPS.map((g) => {
        const mine = (routines ?? []).filter((r) => g.has(r.schedule));
        if (mine.length === 0) return null;
        return (
          <Group key={g.id} title={g.title} count={mine.length}>
      {mine.map((r) => {
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
          </Group>
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
  const [ready, setReady] = useState<string | null>(null);
  function startFrom(t: (typeof READY)[number]) {
    const sc = t.schedule;
    setReady(t.name);
    setName(t.name);
    setInstructions(t.instructions);
    setNotify(t.notify);
    setQuietSkip(t.quietSkip ?? false);
    setKind(kindOf(sc));
    if (sc.kind === "days") {
      setTime(sc.time);
      setDays(sc.days);
    }
    if (sc.kind === "every") setHours(String(sc.hours));
    if (sc.kind === "once") {
      const d = new Date(Date.now() + 3600 * 1000);
      setDate(`${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`);
      setOnceTime(`${pad(d.getHours())}:${pad(d.getMinutes())}`);
    }
    if (sc.kind === "event") setEvent(sc.event);
  }
  const [allow, setAllow] = useState<Allow>(routine?.allow ?? { tools: true, changes: "wait", commands: true });
  const [sayOn, setSayOn] = useState<string[]>(routine?.say_on ?? []);
  const [camera, setCamera] = useState(routine?.camera ?? "");
  const [recognise, setRecognise] = useState(routine?.recognise ?? false);
  const [quietSkip, setQuietSkip] = useState(routine?.quiet_skip ?? false);
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
      const body = { name: name.trim(), persona_id: personaId, instructions: instructions.trim(), schedule: sc, notify, allow, say_on: sayOn, camera, recognise, quiet_skip: quietSkip, ...myZone() };
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
      {!routine && (
        <>
          <P muted>Start from a ready-made one, or write your own (or ask in any chat: "remind me at 5 to…").</P>
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {READY.map((t) => (
              <Chip key={t.name} label={t.name} active={ready === t.name} onPress={() => startFrom(t)} />
            ))}
          </View>
          {ready && READY.find((t) => t.name === ready)?.needs && (
            <P muted>Its persona needs {READY.find((t) => t.name === ready)!.needs}. Change anything in &lt;angle brackets&gt;.</P>
          )}
        </>
      )}
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
      <CameraPick api={api} camera={camera} onChange={setCamera} />
      {(camera || kind === "link") && (
        <View style={{ gap: 4 }}>
          <Chip label={recognise ? "Say who it is: on" : "Say who it is: off"} active={recognise} onPress={() => setRecognise((r) => !r)} />
          <P muted>When the pictures show someone in Known faces (You › People), the persona is told who; anyone else is described.</P>
        </View>
      )}
      {notify !== "never" && (
        <View style={{ gap: 4 }}>
          <Text style={s.label}>In quiet hours</Text>
          <View style={[s.row, { flexWrap: "wrap" }]}>
            <Chip label="Tell me when they end" active={!quietSkip} onPress={() => setQuietSkip(false)} />
            <Chip label="Skip it" active={quietSkip} onPress={() => setQuietSkip(true)} />
          </View>
          <P muted>Skip it for news that's stale by morning, like the doorbell: nothing is sent or said, and the run stays in its chat.</P>
        </View>
      )}
      {notify !== "never" && notify !== "failures" && <SayOutLoud api={api} personaId={personaId} sayOn={sayOn} onChange={setSayOn} />}
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

type Speakers = {
  speakers: { id: string; name: string }[];
  cameras: { id: string; name: string }[];
  home_assistant: boolean;
  problem: string | null;
};

/** A Home Assistant camera whose picture the routine looks at each run. */
function CameraPick({ api, camera, onChange }: { api: Outpost; camera: string; onChange: (c: string) => void }) {
  const [found, setFound] = useState<Speakers | null>(null);
  useEffect(() => {
    api
      .get<Speakers>("/api/speakers")
      .then(setFound)
      .catch(() => setFound(null));
  }, [api]);
  const cameras = found?.cameras ?? [];
  if (!found || (!found.home_assistant && !camera) || (cameras.length === 0 && !camera)) return null;
  return (
    <>
      <Text style={s.label}>Look at a camera (its picture now goes with each run)</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        <Chip label="No camera" active={!camera} onPress={() => onChange("")} />
        {cameras.map((c) => (
          <Chip key={c.id} label={c.name} active={camera === c.id} onPress={() => onChange(c.id)} />
        ))}
        {camera && !cameras.some((c) => c.id === camera) && <Chip label={`${camera} (not in Home Assistant now)`} active onPress={() => onChange("")} />}
      </View>
      {camera ? <P muted>Its persona's model needs to see pictures (a vision model).</P> : null}
    </>
  );
}

/** Where a routine says its result out loud: this phone, and Home Assistant's speakers. */
function SayOutLoud({ api, personaId, sayOn, onChange }: { api: Outpost; personaId: string; sayOn: string[]; onChange: (s: string[]) => void }) {
  const [found, setFound] = useState<Speakers | null>(null);
  const [trying, setTrying] = useState<string | null>(null);
  const [tried, setTried] = useState<{ id: string; error: string | null } | null>(null);
  useEffect(() => {
    api
      .get<Speakers>("/api/speakers")
      .then(setFound)
      .catch((e) => setFound({ speakers: [], cameras: [], home_assistant: false, problem: message(e) }));
  }, [api]);
  const flip = (id: string) => onChange(sayOn.includes(id) ? sayOn.filter((x) => x !== id) : [...sayOn, id]);
  async function tryIt(id: string) {
    setTrying(id);
    setTried(null);
    try {
      await api.json<void>("POST", "/api/speakers/try", { speaker: id, persona_id: personaId || undefined });
      setTried({ id, error: null });
    } catch (e) {
      setTried({ id, error: message(e) });
    } finally {
      setTrying(null);
    }
  }
  const missing = sayOn.filter((id) => id !== "phone" && found && !found.speakers.some((x) => x.id === id));
  return (
    <>
      <Text style={s.label}>Say it out loud too</Text>
      <View style={{ gap: 6 }}>
        <Chip label="On this phone" active={sayOn.includes("phone")} onPress={() => flip("phone")} />
        {found === null ? (
          <P muted>Looking for speakers…</P>
        ) : found.speakers.length > 0 ? (
          found.speakers.map((sp) => (
            <View key={sp.id} style={[s.row, { flexWrap: "wrap", alignItems: "center" }]}>
              <Chip label={sp.name} active={sayOn.includes(sp.id)} onPress={() => flip(sp.id)} />
              <Pressable accessibilityRole="button" disabled={trying !== null} onPress={() => void tryIt(sp.id)} style={{ padding: 8 }}>
                <Text style={{ color: C.muted, textDecorationLine: "underline" }}>{trying === sp.id ? "Saying…" : "Try it"}</Text>
              </Pressable>
              {tried?.id === sp.id && (
                <Text style={{ color: tried.error ? C.danger : C.ok, flexShrink: 1 }}>{tried.error ?? "Sent. Did you hear it?"}</Text>
              )}
            </View>
          ))
        ) : (
          <P muted>
            {found.problem
              ? `Home Assistant's speakers couldn't be listed: ${found.problem}`
              : found.home_assistant
                ? "Home Assistant has no speakers."
                : "To use your speakers, add Home Assistant as a tool (on the PC: Tools)."}
          </P>
        )}
        {missing.map((id) => (
          <Chip key={id} label={`${id} (not in Home Assistant now)`} active onPress={() => flip(id)} />
        ))}
      </View>
      <P muted>
        When it tells you, it also says the result out loud, in the persona's voice. Not during quiet hours or tools-off hours. The phone
        says it when the notification arrives.
      </P>
    </>
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

// You › Wake word: the phone listens for "Hey Jarvis" (on the phone; nothing is sent until it
// hears it), even with the screen off, and what's said next goes to a persona, whose answer
// is spoken. A notification shows while it listens, with Stop.

import { useCallback, useEffect, useState } from "react";
import { Text, View } from "react-native";
import OutpostNet, { type WakeStatus } from "../modules/outpost-net";
import { message, Screen } from "./admin";
import { type Outpost } from "./outpost";
import { type Saved } from "./store";
import { Button, C, Chip, ErrorText, P, s } from "./ui";

const LEVELS: { label: string; percent: number }[] = [
  { label: "Phone's media volume", percent: 0 },
  { label: "At least half", percent: 50 },
  { label: "Loud", percent: 75 },
  { label: "Loudest", percent: 100 },
];

/** How loud it speaks with the app closed (the chime, replies, things said on the phone). */
function VoiceLevel() {
  const [level, setLevel] = useState(() => OutpostNet.voiceLevel?.() ?? 0);
  if (!OutpostNet.setVoiceLevel) return null;
  return (
    <>
      <Text style={s.label}>How loud it speaks</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {LEVELS.map((x) => (
          <Chip
            key={x.label}
            label={x.label}
            active={level === x.percent}
            onPress={() => {
              OutpostNet.setVoiceLevel!(x.percent);
              setLevel(x.percent);
              void OutpostNet.voiceChime?.();
            }}
          />
        ))}
      </View>
      <P muted>
        The chime and replies play at the phone's media volume, which is often turned down when nothing's playing. With a level picked,
        Outpost turns the media volume up to at least that while it speaks, and back afterwards. In the car, it's the car's volume.
      </P>
    </>
  );
}

interface Persona {
  id: string;
  name: string;
}

const SENSITIVITY: { label: string; threshold: number }[] = [
  { label: "Less (fewer false starts)", threshold: 0.7 },
  { label: "Normal", threshold: 0.5 },
  { label: "More (hears it further away)", threshold: 0.35 },
];

function hours(ms: number) {
  const h = ms / 3_600_000;
  return h < 1 ? `${Math.max(1, Math.round(h * 60))} min` : `${h.toFixed(h < 10 ? 1 : 0)} h`;
}

export function WakeScreen({ api, saved, onClose }: { api: Outpost; saved: Saved; onClose: () => void }) {
  const supported = typeof OutpostNet.wakeStatus === "function";
  const [status, setStatus] = useState<WakeStatus | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [word, setWord] = useState("hey_jarvis");
  const [persona, setPersona] = useState<string | null>(null);
  const [threshold, setThreshold] = useState(0.5);
  const [error, setError] = useState<string | null>(null);
  const [check, setCheck] = useState<string | null>(null);

  const refresh = useCallback(() => {
    if (!supported) return;
    const st = OutpostNet.wakeStatus!();
    setStatus(st);
    return st;
  }, [supported]);

  useEffect(() => {
    const st = refresh();
    if (st) {
      setWord(st.word || "hey_jarvis");
      setThreshold(st.threshold || 0.5);
      if (st.persona) setPersona(st.persona);
    }
    api.get<Persona[]>("/api/personas").then(
      (list) => {
        setPersonas(list);
        setPersona((p) => p ?? list.find((x) => x.name.toLowerCase().includes("assistant"))?.id ?? list[0]?.id ?? null);
      },
      (e) => setError(message(e)),
    );
    const t = setInterval(() => void refresh(), 5000);
    return () => clearInterval(t);
  }, [api, refresh]);

  async function start() {
    setError(null);
    try {
      if (!(await OutpostNet.askMicrophone())) {
        setError("Outpost needs the microphone to hear its wake word.");
        return;
      }
      await OutpostNet.askNotifications().catch(() => false);
      if (!persona) {
        setError("Choose the persona that answers.");
        return;
      }
      OutpostNet.wakeStart!(word, persona, threshold, saved.hosts, saved.port, saved.fingerprint, saved.token);
      refresh();
    } catch (e) {
      setError(message(e));
    }
  }

  function stop() {
    OutpostNet.wakeStop?.();
    refresh();
  }

  async function runCheck() {
    setCheck("Checking…");
    try {
      const r = await OutpostNet.wakeCheck!();
      setCheck(
        r.ok
          ? `Working: it heard "Alexa" in the test recording (${Math.round(r.heard * 100)}%) and nothing in another (${Math.round(r.other * 100)}%).`
          : `Something's wrong on this phone: "Alexa" scored ${Math.round(r.heard * 100)}%, other words ${Math.round(r.other * 100)}%.`,
      );
    } catch (e) {
      setCheck(message(e));
    }
  }

  if (!supported)
    return (
      <Screen title="Wake word" onClose={onClose}>
        <P muted>Update the app to use a wake word.</P>
      </Screen>
    );

  const on = !!status?.on;
  const words = status?.words ?? [{ id: "hey_jarvis", name: "Hey Jarvis" }];
  const since = status && status.since > 0 ? Date.now() - status.since : 0;
  const used = status && status.batteryStart >= 0 ? status.batteryStart - status.batteryNow : null;
  return (
    <Screen title="Wake word" onClose={onClose}>
      <P muted>
        Say the word, then what you want, even with the screen off. Listening for it happens on this phone; only what you say after it
        goes to your Outpost.
      </P>
      <View style={[s.card, { gap: 8 }]}>
        <Text style={{ color: on ? C.ok : C.muted, fontWeight: "600" }}>
          {on ? `Listening for "${words.find((w) => w.id === status?.word)?.name ?? "Hey Jarvis"}"` : "Off"}
        </Text>
        {on && since > 0 && (
          <Text style={{ color: C.soft, fontSize: 13 }}>
            For {hours(since)}
            {used != null && !status?.charging ? `, battery down ${Math.max(0, used)}% since then (everything on the phone, not just this)` : ""}
            {status?.charging ? " (charging now)" : ""} · heard {status?.wakes ?? 0} time{status?.wakes === 1 ? "" : "s"}
            {status?.ignored ? ` · ignored ${status.ignored} that weren't the word (TV, talk)` : ""}
          </Text>
        )}
        {!!status?.error && <Text style={{ color: C.danger, fontSize: 13 }}>{status.error}</Text>}
        {on ? <Button title="Stop listening" onPress={stop} /> : <Button title="Start listening" onPress={() => void start()} />}
      </View>

      <Text style={s.label}>The word</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {words.map((w) => (
          <Chip key={w.id} label={w.name} active={word === w.id} onPress={() => setWord(w.id)} />
        ))}
      </View>
      <Text style={s.label}>Who answers</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {personas.map((p) => (
          <Chip key={p.id} label={p.name} active={persona === p.id} onPress={() => setPersona(p.id)} />
        ))}
      </View>
      <Text style={s.label}>How easily it wakes</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {SENSITIVITY.map((x) => (
          <Chip key={x.label} label={x.label} active={Math.abs(threshold - x.threshold) < 0.01} onPress={() => setThreshold(x.threshold)} />
        ))}
      </View>
      {on && <P muted>Changes take effect when you stop and start it again.</P>}
      <VoiceLevel />
      <P muted>
        It chimes when it hears the word (two rising notes): then say what you want. Two falling notes mean it couldn't reach your
        Outpost or didn't get that. Before it chimes, your Outpost checks the word was really said
        (TV can sound like it to the phone), so the chime comes about half a second after the word. While it's on, news set to be said on this phone (a routine, the
        doorbell) is said as soon as it's ready. Anything that asks first (a lock, a door, an alarm) is asked aloud: say
        yes or no. The talk is kept in a chat called "Phone voice". Talk and the Mic button pause it while they're in use. After the phone
        restarts, Android doesn't let it start by itself: it comes back on when you next open Outpost (unless you stopped it).
      </P>
      <P muted>
        On Samsung phones, set Outpost's battery use to Unrestricted (Settings › Apps › Outpost › Battery), or it may stop listening
        with the screen off.
      </P>
      <Button kind="ghost" title="Check the wake word works on this phone" onPress={() => void runCheck()} />
      {check && <Text style={{ color: C.soft, fontSize: 13 }}>{check}</Text>}
      <P muted>Wake words by openWakeWord (David Scripka): free for non-commercial use.</P>
      <ErrorText>{error}</ErrorText>
    </Screen>
  );
}

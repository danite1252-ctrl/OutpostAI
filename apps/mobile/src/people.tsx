// People and devices on the phone. Everyone sees their own paired phones (and can pair
// another of theirs). Owners also see everyone's accounts and phones, invite people (a QR
// code, or a link sent with any app), reset a PIN, and remove accounts.

import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Alert, Share, Text, View } from "react-native";
import { SvgXml } from "react-native-svg";
import { ListState, message, Screen, Section, usePin } from "./admin";
import type { Outpost } from "./outpost";
import { Avatar, Button, C, Chip, ErrorText, Field, P, s } from "./ui";

interface Account {
  id: string;
  name: string;
  role: "owner" | "member" | "restricted" | "service";
  avatar_color: string;
}
interface Device {
  id: string;
  account_id: string;
  label: string;
  last_used_at: number;
  account_name: string;
  current: boolean;
}
interface PairingView {
  id: string;
  code: string;
  expires_at: number;
  words: string[];
  link: string;
  qr_svg: string;
  name: string;
  invite?: { kind: "join" | "reset"; name: string; role: string | null } | null;
}

const ROLES: { id: Account["role"]; label: string; hint: string }[] = [
  { id: "member", label: "Member", hint: "Chats, makes pictures, adds tools at an address." },
  { id: "owner", label: "Owner", hint: "Everything, including models, people and the server." },
  { id: "restricted", label: "Child", hint: "Restricted: only what owners allow." },
];

function ago(unix: number) {
  const mins = Math.max(0, Math.round((Date.now() / 1000 - unix) / 60));
  if (mins < 2) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const h = Math.round(mins / 60);
  if (h < 48) return `${h} h ago`;
  return `${Math.round(h / 24)} days ago`;
}

export function PeopleScreen({ api, isOwner, myAccountId, onClose }: { api: Outpost; isOwner: boolean; myAccountId: string; onClose: () => void }) {
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [devices, setDevices] = useState<Device[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [code, setCode] = useState<PairingView | null>(null);
  const [inviting, setInviting] = useState(false);
  const { guard, prompt } = usePin();

  const reload = useCallback(async () => {
    try {
      setDevices(await api.get<Device[]>(`/api/devices${isOwner ? "?all=true" : ""}`));
      if (isOwner) setAccounts(await api.get<Account[]>("/api/accounts"));
    } catch (e) {
      setError(message(e));
    }
  }, [api, isOwner]);
  useEffect(() => {
    void reload();
  }, [reload]);

  async function run(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(message(e));
    }
  }
  const pairMine = () =>
    run(async () => {
      const v = await guard((h) => api.json<PairingView>("POST", "/api/pairing", undefined, h));
      if (v) setCode(v);
    });
  const resetPin = (a: Account) =>
    run(async () => {
      const v = await guard((h) => api.json<PairingView>("POST", "/api/invites", { account_id: a.id }, h));
      if (v) setCode(v);
    });
  function removeAccount(a: Account) {
    Alert.alert(`Delete ${a.name}'s account?`, "Their chats, pictures and phones go with it.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: () => void run(() => guard((h) => api.json("DELETE", `/api/accounts/${a.id}`, undefined, h)).then(reload)),
      },
    ]);
  }
  function unpair(d: Device) {
    const whose = d.account_id === myAccountId ? "" : ` (${d.account_name}'s)`;
    Alert.alert(`Unpair "${d.label}"${whose}?`, "It's signed out at once and must pair again.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Unpair",
        style: "destructive",
        onPress: () => void run(() => guard((h) => api.json("DELETE", `/api/devices/${d.id}`, undefined, h)).then(reload)),
      },
    ]);
  }

  if (code) return <CodeScreen view={code} onClose={() => { setCode(null); void reload(); }} />;
  if (inviting)
    return (
      <InviteScreen
        api={api}
        guard={guard}
        prompt={prompt}
        onMade={(v) => {
          setInviting(false);
          setCode(v);
        }}
        onClose={() => setInviting(false)}
      />
    );

  return (
    <Screen title={isOwner ? "People and devices" : "Your devices"} onClose={onClose}>
      {prompt}
      <ErrorText>{error}</ErrorText>
      {isOwner && (
        <Section title="People">
          <Button title="Invite someone" onPress={() => setInviting(true)} />
          <ListState loading={accounts === null} error={null} empty={false} emptyText="" />
          {(accounts ?? [])
            .filter((a) => a.role !== "service")
            .map((a) => (
              <View key={a.id} style={[s.card, { gap: 8 }]}>
                <View style={s.row}>
                  <Avatar name={a.name} color={a.avatar_color} size={32} />
                  <View style={{ flex: 1 }}>
                    <Text style={{ color: C.ink, fontWeight: "600" }}>
                      {a.name}
                      {a.id === myAccountId ? " (you)" : ""}
                    </Text>
                    <Text style={{ color: C.muted, fontSize: 13 }}>{ROLES.find((r) => r.id === a.role)?.label ?? a.role}</Text>
                  </View>
                </View>
                <View style={[s.row, { flexWrap: "wrap" }]}>
                  <Chip label="New PIN (send a link)" onPress={() => void resetPin(a)} />
                  {a.id !== myAccountId && <Chip label="Delete" color={C.danger} onPress={() => removeAccount(a)} />}
                </View>
                {a.role !== "owner" && <ToolHoursRow api={api} id={a.id} name={a.name} />}
              </View>
            ))}
        </Section>
      )}
      <Section title={isOwner ? "Paired phones and devices" : "Paired with your account"}>
        <Button kind="ghost" title="Pair another device of mine" onPress={() => void pairMine()} />
        <ListState loading={devices === null} error={null} empty={devices?.length === 0} emptyText="None yet." />
        {(devices ?? []).map((d) => (
          <View key={d.id} style={[s.card, s.row]}>
            <View style={{ flex: 1 }}>
              <Text style={{ color: C.ink }}>
                {d.label}
                {d.current ? " (this phone)" : ""}
              </Text>
              <Text style={{ color: C.muted, fontSize: 13 }}>
                {isOwner ? `${d.account_name} · ` : ""}used {ago(d.last_used_at)}
              </Text>
            </View>
            {!d.current && <Chip label="Unpair" color={C.danger} onPress={() => unpair(d)} />}
          </View>
        ))}
      </Section>
    </Screen>
  );
}

function InviteScreen({
  api,
  guard,
  prompt,
  onMade,
  onClose,
}: {
  api: Outpost;
  guard: ReturnType<typeof usePin>["guard"];
  prompt: ReactNode;
  onMade: (v: PairingView) => void;
  onClose: () => void;
}) {
  const [name, setName] = useState("");
  const [role, setRole] = useState<Account["role"]>("member");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  async function make() {
    setBusy(true);
    setError(null);
    try {
      const v = await guard((h) => api.json<PairingView>("POST", "/api/invites", { name: name.trim(), role }, h));
      if (v) onMade(v);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Screen title="Invite someone" onClose={onClose} back="‹ Back">
      {prompt}
      <P muted>They scan the code (or open the link) with Outpost on their own phone, and pick their own PIN. It works once, for 24 hours.</P>
      <Field label="Their name" value={name} onChangeText={setName} />
      {ROLES.map((r) => (
        <View key={r.id} style={{ gap: 2 }}>
          <View style={s.row}>
            <Chip label={r.label} active={role === r.id} onPress={() => setRole(r.id)} />
            <Text style={{ color: C.muted, fontSize: 13, flex: 1 }}>{r.hint}</Text>
          </View>
        </View>
      ))}
      <ErrorText>{error}</ErrorText>
      <Button title="Make the invite" disabled={!name.trim()} busy={busy} onPress={() => void make()} />
    </Screen>
  );
}

/** A pairing code or invite: the QR to scan, and the link to send. */
function CodeScreen({ view, onClose }: { view: PairingView; onClose: () => void }) {
  const invite = view.invite;
  const title = invite ? (invite.kind === "reset" ? `New PIN for ${invite.name}` : `Invite for ${invite.name}`) : "Pair a device";
  const until = new Date(view.expires_at * 1000).toLocaleString();
  return (
    <Screen title={title} onClose={onClose} back="Done">
      <P muted>
        {invite
          ? "Scan this with Outpost on their phone (Pair › Scan), or send them the link. They pick their own PIN."
          : "On another phone, open Outpost › Pair and scan this. On a PC, choose Connect to an Outpost server, type the server's address, then this code."}
      </P>
      <View style={{ alignSelf: "center", backgroundColor: "#fff", padding: 12, borderRadius: 12 }}>
        <SvgXml xml={view.qr_svg} width={240} height={240} />
      </View>
      <Text style={{ color: C.ink, fontSize: 28, fontWeight: "700", textAlign: "center", letterSpacing: 4 }}>{view.code}</Text>
      <P muted style={{ textAlign: "center" }}>
        Check words: {view.words.join(" · ")} · works until {until}
      </P>
      <Button kind="ghost" title="Send the link" onPress={() => void Share.share({ message: `Join Outpost (${view.name}): ${view.link}` })} />
    </Screen>
  );
}

/** Hours when someone's tools that do things are off (look-ups still work): bedtime for the kids. */
function ToolHoursRow({ api, id, name }: { api: Outpost; id: string; name: string }) {
  type Off = { from: string; to: string } | null;
  const [open, setOpen] = useState(false);
  const [off, setOff] = useState<Off | undefined>(undefined);
  const [from, setFrom] = useState("21:00");
  const [to, setTo] = useState("07:00");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (!open) return;
    api.get<{ off: Off }>(`/api/accounts/${id}/tool-hours`).then(
      (h) => {
        setOff(h.off);
        if (h.off) {
          setFrom(h.off.from);
          setTo(h.off.to);
        }
      },
      (e) => setError(message(e)),
    );
  }, [open, api, id]);
  async function save(next: Off) {
    setError(null);
    try {
      setOff((await api.json<{ off: Off }>("PUT", `/api/accounts/${id}/tool-hours`, { off: next })).off);
    } catch (e) {
      setError(message(e));
    }
  }
  if (!open) return <Chip label="Tool hours…" onPress={() => setOpen(true)} />;
  if (off === undefined) return <P muted>{error ?? "Loading…"}</P>;
  const changed = !!off && (off.from !== from.trim() || off.to !== to.trim());
  return (
    <View style={{ gap: 6 }}>
      <Chip
        label={off ? `${name}'s tools are off ${off.from} to ${off.to}` : "Tools any time"}
        active={!!off}
        onPress={() => void save(off ? null : { from: from.trim(), to: to.trim() })}
      />
      <View style={s.row}>
        <View style={{ flex: 1 }}>
          <Field label="Off from" value={from} onChangeText={setFrom} keyboardType="numbers-and-punctuation" />
        </View>
        <View style={{ flex: 1 }}>
          <Field label="Until" value={to} onChangeText={setTo} keyboardType="numbers-and-punctuation" />
        </View>
      </View>
      {changed && <Button title="Save tool hours" onPress={() => void save({ from: from.trim(), to: to.trim() })} />}
      <P muted>Then a persona can't turn things on or off for them; look-ups still work.</P>
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

import { CameraView, useCameraPermissions } from "expo-camera";
import { useEffect, useState } from "react";
import { Platform, Pressable, ScrollView, Text, View } from "react-native";
import OutpostNet, { type FoundService } from "../../modules/outpost-net";
import { ApiError, Outpost, type Account, type HelloResponse, type SignInResponse } from "../outpost";
import { parsePairLink, type Saved } from "../store";
import { Avatar, Button, C, ErrorText, Field, H1, P, s } from "../ui";
import { checkWords } from "../words";

type Target = { name: string; hosts: string[]; port: number; fingerprint: string };

type Step =
  | { kind: "find" }
  | { kind: "scan" }
  | { kind: "manual" }
  | { kind: "verify"; target: Target }
  | { kind: "accounts"; target: Target; code: string; hello: HelloResponse }
  | { kind: "pin"; target: Target; code: string; hello: HelloResponse; account: Account };

function defaultDevice() {
  const c = Platform.constants as { Manufacturer?: string; Model?: string; Brand?: string };
  const maker = (c.Manufacturer ?? c.Brand ?? "").trim();
  const name = `${maker ? maker[0].toUpperCase() + maker.slice(1) : "Android"} ${c.Model ?? "phone"}`.trim();
  return name.slice(0, 60);
}

function message(e: unknown) {
  return e instanceof ApiError || e instanceof Error ? e.message : "Something went wrong.";
}

/** Find the Outpost, check it's the real one, pick an account, enter its PIN. */
export default function Pair({ onPaired }: { onPaired: (s: Saved) => void }) {
  const [step, setStep] = useState<Step>({ kind: "find" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const go = (next: Step) => {
    setError(null);
    setStep(next);
  };

  /** Read the certificate at an address and move on to checking its words. */
  async function reach(name: string, host: string, port: number) {
    setBusy(true);
    setError(null);
    try {
      const fingerprint = await OutpostNet.fingerprint(host, port);
      go({ kind: "verify", target: { name, hosts: [host], port, fingerprint } });
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  /** The QR carries the fingerprint, so the certificate is checked against it directly. */
  async function scanned(data: string) {
    const link = parsePairLink(data);
    if (!link) {
      setError("That isn't an Outpost pairing code.");
      return go({ kind: "find" });
    }
    setBusy(true);
    setError(null);
    for (const host of link.hosts) {
      try {
        const fp = await OutpostNet.fingerprint(host, link.port);
        if (fp.toLowerCase() !== link.fingerprint) {
          setBusy(false);
          setError("The PC at that address isn't the one that showed this code. Don't pair; check your network.");
          return go({ kind: "find" });
        }
        const target = { name: link.name, hosts: [host, ...link.hosts.filter((h) => h !== host)], port: link.port, fingerprint: fp };
        const hello = await new Outpost(target).json<HelloResponse>("POST", "/api/pair/hello", { code: link.code });
        setBusy(false);
        return go({ kind: "accounts", target, code: link.code, hello });
      } catch (e) {
        setError(message(e));
      }
    }
    setBusy(false);
    go({ kind: "find" });
  }

  switch (step.kind) {
    case "find":
      return <Find busy={busy} error={error} onReach={reach} onScan={() => go({ kind: "scan" })} onManual={() => go({ kind: "manual" })} />;
    case "scan":
      return <Scan onScanned={(d) => void scanned(d)} onCancel={() => go({ kind: "find" })} />;
    case "manual":
      return <Manual busy={busy} error={error} onReach={(h, p) => void reach("Outpost", h, p)} onBack={() => go({ kind: "find" })} />;
    case "verify":
      return (
        <Verify
          target={step.target}
          onBack={() => go({ kind: "find" })}
          onHello={(code, hello) => go({ kind: "accounts", target: step.target, code, hello })}
        />
      );
    case "accounts":
      if (step.hello.invite) {
        return <NewPin step={step} invite={step.hello.invite} onBack={() => go({ kind: "find" })} onPaired={onPaired} />;
      }
      return (
        <View style={s.screen}>
          <ScrollView contentContainerStyle={s.pad}>
            <H1>Who are you?</H1>
            <P muted>On {step.hello.name}. Pick your account; you'll enter its PIN next.</P>
            {step.hello.accounts.map((a) => (
              <Pressable
                key={a.id}
                accessibilityRole="button"
                onPress={() => go({ kind: "pin", target: step.target, code: step.code, hello: step.hello, account: a })}
                style={({ pressed }) => [s.card, s.row, pressed && { backgroundColor: C.raised }]}
              >
                <Avatar name={a.name} color={a.avatar_color} size={40} />
                <View>
                  <Text style={{ color: C.ink, fontSize: 17, fontWeight: "600" }}>{a.name}</Text>
                  <Text style={{ color: C.muted, textTransform: "capitalize" }}>{a.role}</Text>
                </View>
              </Pressable>
            ))}
            <Button kind="ghost" title="Start again" onPress={() => go({ kind: "find" })} />
          </ScrollView>
        </View>
      );
    case "pin":
      return <Pin step={step} onBack={() => go({ ...step, kind: "accounts" })} onPaired={onPaired} />;
  }
}

function Find({
  busy,
  error,
  onReach,
  onScan,
  onManual,
}: {
  busy: boolean;
  error: string | null;
  onReach: (name: string, host: string, port: number) => void;
  onScan: () => void;
  onManual: () => void;
}) {
  const [found, setFound] = useState<FoundService[]>([]);
  useEffect(() => {
    const subs = [
      OutpostNet.addListener("onService", (svc) => {
        if (!svc.host) return;
        setFound((list) => [...list.filter((f) => f.name !== svc.name), svc]);
      }),
      OutpostNet.addListener("onServiceLost", ({ name }) => setFound((list) => list.filter((f) => f.name !== name))),
    ];
    try {
      OutpostNet.startDiscovery();
    } catch {
      // No discovery on this phone: scanning and typing still work.
    }
    return () => {
      subs.forEach((x) => x.remove());
      OutpostNet.stopDiscovery();
    };
  }, []);

  return (
    <View style={s.screen}>
      <ScrollView contentContainerStyle={s.pad}>
        <H1>Pair with your Outpost</H1>
        <P muted>On the Outpost PC, or any paired device, open Connections › Pair a device. Keep the phone on the same Wi-Fi.</P>
        <Button title="Scan the code on the PC" onPress={onScan} />
        <Text style={[s.label, { marginTop: 8 }]}>On this network</Text>
        {found.length === 0 ? (
          <P muted>Looking…</P>
        ) : (
          found.map((f) => (
            <Pressable
              key={f.name}
              accessibilityRole="button"
              disabled={busy}
              onPress={() => onReach(f.txt.name || f.name, f.host, f.port)}
              style={({ pressed }) => [s.card, pressed && { backgroundColor: C.raised }]}
            >
              <Text style={{ color: C.ink, fontSize: 17, fontWeight: "600" }}>{f.txt.name || f.name}</Text>
              <Text style={{ color: C.muted }}>
                {f.host}:{f.port}
                {f.txt.version ? ` · Outpost ${f.txt.version}` : ""}
              </Text>
            </Pressable>
          ))
        )}
        <Button kind="ghost" title="Type the address" onPress={onManual} disabled={busy} />
        {busy && <P muted>Connecting…</P>}
        <ErrorText>{error}</ErrorText>
      </ScrollView>
    </View>
  );
}

function Scan({ onScanned, onCancel }: { onScanned: (data: string) => void; onCancel: () => void }) {
  const [permission, ask] = useCameraPermissions();
  const [done, setDone] = useState(false);
  if (!permission) return <View style={s.screen} />;
  if (!permission.granted) {
    return (
      <View style={[s.screen, s.pad]}>
        <H1>Camera</H1>
        <P muted>Outpost needs the camera only to read the pairing code on your PC.</P>
        <Button title="Allow the camera" onPress={() => void ask()} />
        <Button kind="ghost" title="Back" onPress={onCancel} />
      </View>
    );
  }
  return (
    <View style={s.screen}>
      <CameraView
        style={{ flex: 1 }}
        facing="back"
        barcodeScannerSettings={{ barcodeTypes: ["qr"] }}
        onBarcodeScanned={
          done
            ? undefined
            : ({ data }) => {
                setDone(true);
                onScanned(data);
              }
        }
      />
      <View style={s.pad}>
        <P muted>Point the camera at the QR code in the Pair a device window.</P>
        <Button kind="ghost" title="Cancel" onPress={onCancel} />
      </View>
    </View>
  );
}

function Manual({
  busy,
  error,
  onReach,
  onBack,
}: {
  busy: boolean;
  error: string | null;
  onReach: (host: string, port: number) => void;
  onBack: () => void;
}) {
  const [host, setHost] = useState("");
  const [port, setPort] = useState("7421");
  return (
    <View style={[s.screen, s.pad]}>
      <H1>Type the address</H1>
      <P muted>It's under the code in the Pair a device window, e.g. 192.168.1.20:7421.</P>
      <Field label="PC address" value={host} onChangeText={setHost} autoCapitalize="none" autoCorrect={false} keyboardType="url" placeholder="192.168.1.20" />
      <Field label="Port" value={port} onChangeText={setPort} keyboardType="number-pad" />
      <Button title="Connect" busy={busy} disabled={!host.trim()} onPress={() => onReach(host.trim(), Number(port) || 7421)} />
      <Button kind="ghost" title="Back" onPress={onBack} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

/** Show the three words from the certificate this phone sees; the PC shows its own. */
function Verify({ target, onBack, onHello }: { target: Target; onBack: () => void; onHello: (code: string, hello: HelloResponse) => void }) {
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const words = checkWords(target.fingerprint);

  async function send() {
    setBusy(true);
    setError(null);
    try {
      const clean = code.replace(/\D/g, "");
      const hello = await new Outpost(target).json<HelloResponse>("POST", "/api/pair/hello", { code: clean });
      onHello(clean, hello);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <View style={[s.screen, s.pad]}>
      <H1>Check the words</H1>
      <P muted>The Pair a device window on your PC shows three words. They must be exactly these:</P>
      <View style={[s.card, { alignItems: "center" }]}>
        <Text style={{ color: C.amber, fontSize: 24, fontWeight: "700", letterSpacing: 1 }}>{words.join(" · ")}</Text>
      </View>
      <P muted>Different words? Stop: something on the network may be pretending to be your PC.</P>
      <Field label="Code shown on the PC" value={code} onChangeText={setCode} keyboardType="number-pad" maxLength={7} placeholder="123 456" />
      <Button title="The words match, continue" busy={busy} disabled={code.replace(/\D/g, "").length !== 6} onPress={() => void send()} />
      <Button kind="ghost" title="Back" onPress={onBack} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

function Pin({
  step,
  onBack,
  onPaired,
}: {
  step: Extract<Step, { kind: "pin" }>;
  onBack: () => void;
  onPaired: (s: Saved) => void;
}) {
  const [pin, setPin] = useState("");
  const [numeric, setNumeric] = useState(true);
  const [device, setDevice] = useState(defaultDevice);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function pair() {
    setBusy(true);
    setError(null);
    try {
      const r = await new Outpost(step.target).json<SignInResponse>("POST", "/api/pair/complete", {
        code: step.code,
        account_id: step.account.id,
        secret: pin,
        device_name: device.trim() || "Phone",
      });
      onPaired({
        ...step.target,
        name: step.hello.name,
        token: r.token,
        account: r.account,
        sessionId: r.session.id,
        device: r.session.label,
      });
    } catch (e) {
      if (e instanceof ApiError && e.attemptsLeft != null) setError(`Wrong PIN. ${e.attemptsLeft} tries left before the account locks.`);
      else setError(message(e));
      setPin("");
    } finally {
      setBusy(false);
    }
  }

  return (
    <View style={[s.screen, s.pad]}>
      <View style={s.row}>
        <Avatar name={step.account.name} color={step.account.avatar_color} size={44} />
        <H1>{step.account.name}</H1>
      </View>
      <Field
        label={numeric ? "PIN" : "Password"}
        value={pin}
        onChangeText={setPin}
        secureTextEntry
        autoFocus
        autoCapitalize="none"
        keyboardType={numeric ? "number-pad" : "default"}
      />
      <Pressable accessibilityRole="button" onPress={() => setNumeric((n) => !n)}>
        <Text style={{ color: C.ember }}>{numeric ? "I use a password instead" : "I use a PIN"}</Text>
      </Pressable>
      <Field label="Name this phone" value={device} onChangeText={setDevice} maxLength={60} />
      <Button title="Pair" busy={busy} disabled={!pin} onPress={() => void pair()} />
      <Button kind="ghost" title="Back" onPress={onBack} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

/** An invite: the person picks their own PIN, which makes their account (or resets it). */
function NewPin({
  step,
  invite,
  onBack,
  onPaired,
}: {
  step: Extract<Step, { kind: "accounts" }>;
  invite: NonNullable<HelloResponse["invite"]>;
  onBack: () => void;
  onPaired: (s: Saved) => void;
}) {
  const [pin, setPin] = useState("");
  const [again, setAgain] = useState("");
  const [numeric, setNumeric] = useState(true);
  const [device, setDevice] = useState(defaultDevice);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  async function join() {
    if (pin !== again) return setError("The two entries don't match.");
    const ok = numeric ? /^\d{4,12}$/.test(pin) : pin.length >= 6;
    if (!ok) return setError(numeric ? "A PIN is 4 to 12 digits." : "A password needs at least 6 characters.");
    setBusy(true);
    setError(null);
    try {
      const r = await new Outpost(step.target).json<SignInResponse>("POST", "/api/pair/accept", {
        code: step.code,
        secret: pin,
        device_name: device.trim() || "Phone",
      });
      onPaired({
        ...step.target,
        name: step.hello.name,
        token: r.token,
        account: r.account,
        sessionId: r.session.id,
        device: r.session.label,
      });
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <View style={[s.screen, s.pad]}>
      <H1>{invite.kind === "join" ? `Welcome, ${invite.name}` : `New PIN for ${invite.name}`}</H1>
      <P muted>
        {invite.kind === "join"
          ? `You're joining ${step.hello.name}. Pick your own PIN: this phone stays signed in, and asks for it before big changes.`
          : "Pick a new PIN. It replaces the old one, and your other devices will need to sign in again."}
      </P>
      <Field
        label={numeric ? "PIN" : "Password"}
        value={pin}
        onChangeText={setPin}
        secureTextEntry
        autoFocus
        autoCapitalize="none"
        keyboardType={numeric ? "number-pad" : "default"}
      />
      <Field
        label="Again"
        value={again}
        onChangeText={setAgain}
        secureTextEntry
        autoCapitalize="none"
        keyboardType={numeric ? "number-pad" : "default"}
      />
      <Pressable accessibilityRole="button" onPress={() => setNumeric((n) => !n)}>
        <Text style={{ color: C.ember }}>{numeric ? "Use a password instead" : "Use a PIN"}</Text>
      </Pressable>
      <Field label="Name this phone" value={device} onChangeText={setDevice} maxLength={60} />
      <Button title={invite.kind === "join" ? "Join" : "Save my new PIN"} busy={busy} disabled={!pin || !again} onPress={() => void join()} />
      <Button kind="ghost" title="Back" onPress={onBack} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

import { useEffect, useState } from "react";
import { Alert, Linking, ScrollView, Text, View } from "react-native";
import { checkWords } from "../words";
import { APP_VERSION } from "../version";
import type { Outpost } from "../outpost";
import { loadVoiceCheck, saveVoiceCheck, type Saved } from "../store";
import { Avatar, Button, C, Chip, ErrorText, H1, P, s } from "../ui";

export default function Me({ api, saved, onSignedOut }: { api: Outpost; saved: Saved; onSignedOut: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const [voiceCheck, setVoiceCheck] = useState(false);
  useEffect(() => {
    void loadVoiceCheck().then(setVoiceCheck);
  }, []);
  // The PC checks GitHub for releases; a newer phone app is shown here (downloading it is up to you).
  const [newer, setNewer] = useState<{ version: string; url: string } | null>(null);
  useEffect(() => {
    api
      .get<{ mobile: { version: string; url: string } | null }>("/api/updates")
      .then((u) => setNewer(u.mobile && isNewer(u.mobile.version, APP_VERSION) ? u.mobile : null))
      .catch(() => undefined);
  }, [api]);

  function unpair() {
    Alert.alert("Unpair this phone?", "It signs out of Outpost. You can pair it again from the PC.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Unpair",
        style: "destructive",
        onPress: async () => {
          try {
            await api.json("DELETE", `/api/devices/${saved.sessionId}`);
          } catch (e) {
            // Unreachable or already unpaired: forget it here anyway.
            setError(e instanceof Error ? e.message : null);
          }
          onSignedOut();
        },
      },
    ]);
  }

  return (
    <ScrollView style={s.screen} contentContainerStyle={s.pad}>
      <View style={s.row}>
        <Avatar name={saved.account.name} color={saved.account.avatar_color} size={48} />
        <View>
          <H1>{saved.account.name}</H1>
          <Text style={{ color: C.muted, textTransform: "capitalize" }}>{saved.account.role}</Text>
        </View>
      </View>
      <View style={s.card}>
        <P>
          Paired with <Text style={{ fontWeight: "700" }}>{saved.name}</Text> as "{saved.device}".
        </P>
        <P muted>
          {api.conn.hosts[0]}:{saved.port} · encrypted · check words {checkWords(saved.fingerprint).join(" · ")}
        </P>
      </View>
      <P muted>
        This phone only talks to that PC, checked by its certificate. Risky things (deleting models, making app keys,
        changing accounts) ask for your PIN again.
      </P>
      {newer && (
        <View style={[s.card, { borderColor: C.ember }]}>
          <Text style={{ color: C.ink, fontSize: 16, fontWeight: "600" }}>Outpost {newer.version} for this phone is out</Text>
          <P muted>You have {APP_VERSION}. Download it, open it, and Android installs it over this one (you stay paired).</P>
          <Button title="Download the new app" onPress={() => void Linking.openURL(newer.url)} />
        </View>
      )}
      <View style={s.card}>
        <Text style={{ color: C.ink, fontSize: 16, fontWeight: "600" }}>Troubleshooting</Text>
        <P muted>
          Voice check line: shows the microphone and room levels on the hands-free screen, and what happened to the
          last thing you said. Useful if hands-free doesn't hear you or doesn't answer.
        </P>
        <View style={s.row}>
          <Chip
            label={voiceCheck ? "Voice check line: on" : "Voice check line: off"}
            active={voiceCheck}
            onPress={() => {
              const on = !voiceCheck;
              setVoiceCheck(on);
              void saveVoiceCheck(on);
            }}
          />
        </View>
      </View>
      <Button kind="danger" title="Unpair this phone" onPress={unpair} />
      <P muted>Outpost for Android {APP_VERSION}</P>
      <ErrorText>{error}</ErrorText>
    </ScrollView>
  );
}

/** `a` is a newer version than `b` (0.10.0 > 0.9.2). */
function isNewer(a: string, b: string) {
  const parts = (v: string) => v.replace(/^v/, "").split("-")[0].split(".").map((n) => parseInt(n, 10) || 0);
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
}

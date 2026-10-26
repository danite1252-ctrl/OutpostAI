import { useEffect, useState } from "react";
import { Alert, ScrollView, Text, View } from "react-native";
import { checkWords } from "../words";
import { APP_VERSION } from "../version";
import type { Outpost } from "../outpost";
import { loadAutoUpdate, loadVoiceCheck, saveAutoUpdate, saveVoiceCheck, type Saved } from "../store";
import type { usePhoneUpdate } from "../updater";
import { Avatar, Button, C, Chip, ErrorText, H1, P, s } from "../ui";

export default function Me({
  api,
  saved,
  onSignedOut,
  update,
}: {
  api: Outpost;
  saved: Saved;
  onSignedOut: () => void;
  update: ReturnType<typeof usePhoneUpdate>;
}) {
  const [error, setError] = useState<string | null>(null);
  const [voiceCheck, setVoiceCheck] = useState(false);
  useEffect(() => {
    void loadVoiceCheck().then(setVoiceCheck);
  }, []);
  const [auto, setAuto] = useState(true);
  useEffect(() => {
    void loadAutoUpdate().then(setAuto);
  }, []);
  const u = update.state;

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
      <View style={s.card}>
        <Text style={{ color: C.ink, fontSize: 16, fontWeight: "600" }}>Updates</Text>
        <P muted>
          {u.phase === "none"
            ? `Outpost ${APP_VERSION} for Android: up to date.`
            : u.phase === "available"
              ? `Outpost ${u.version} is out (you have ${APP_VERSION}).`
              : u.phase === "downloading"
                ? `Downloading Outpost ${u.version}… ${u.percent}%`
                : u.phase === "ready"
                  ? `Outpost ${u.version} is downloaded and ready.${u.silent && auto ? " It installs by itself the next time you leave the app." : ""}`
                  : u.phase === "permission"
                    ? `Outpost ${u.version} is ready. Android needs you to allow Outpost to install its updates (once).`
                    : u.phase === "installing"
                      ? `Installing Outpost ${u.version}…`
                      : `Outpost ${u.version}: ${u.message}`}
        </P>
        {u.phase === "available" && <Button title="Download it" onPress={update.download} />}
        {(u.phase === "ready" || u.phase === "error") && <Button title="Restart to update" onPress={() => void update.install()} />}
        {u.phase === "permission" && <Button title="Allow updates" onPress={update.allow} />}
        <View style={s.row}>
          <Chip
            label={auto ? "Install updates automatically: on" : "Install updates automatically: off"}
            active={auto}
            onPress={() => {
              const on = !auto;
              setAuto(on);
              void saveAutoUpdate(on);
            }}
          />
        </View>
        <P muted>
          Updates come from Outpost's GitHub releases (about 90 MB) and only install if they're signed with the same key
          as this app. You stay paired.
        </P>
      </View>
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

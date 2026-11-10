// "What's new": after the app updates, the notes of each release since the last one this phone
// showed, once. Nothing on a first install, and nothing if GitHub can't be reached.

import { useEffect, useState } from "react";
import { Modal, ScrollView, Text, View } from "react-native";
import type { Outpost } from "./outpost";
import { isNewer } from "./updater";
import { loadWhatsNewSeen, saveWhatsNewSeen } from "./store";
import { APP_VERSION } from "./version";
import { Button, C, s } from "./ui";

interface ReleaseNotes {
  version: string;
  notes: string;
  date: string | null;
}

export function WhatsNew({ api }: { api: Outpost }) {
  const [notes, setNotes] = useState<ReleaseNotes[] | null>(null);

  useEffect(() => {
    let gone = false;
    void (async () => {
      const last = await loadWhatsNewSeen();
      if (!last) return saveWhatsNewSeen(APP_VERSION);
      if (!isNewer(APP_VERSION, last)) return;
      try {
        const list = await api.get<ReleaseNotes[]>(
          `/api/updates/notes?since=${encodeURIComponent(last)}&upto=${encodeURIComponent(APP_VERSION)}`,
        );
        if (gone) return;
        if (list.length > 0) setNotes(list);
        else void saveWhatsNewSeen(APP_VERSION);
      } catch {
        // Not reachable now: try again next time.
      }
    })();
    return () => {
      gone = true;
    };
  }, [api]);

  if (!notes) return null;
  const close = () => {
    void saveWhatsNewSeen(APP_VERSION);
    setNotes(null);
  };
  return (
    <Modal visible transparent animationType="fade" onRequestClose={close}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "center", padding: 24 }}>
        <View style={[s.card, { gap: 12, maxHeight: "80%" }]}>
          <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>What's new in Outpost {APP_VERSION}</Text>
          <ScrollView contentContainerStyle={{ gap: 14 }}>
            {notes.map((n) => (
              <View key={n.version} style={{ gap: 4 }}>
                {notes.length > 1 && <Text style={{ color: C.muted, fontSize: 13 }}>{n.version}</Text>}
                <Text style={{ color: C.soft, fontSize: 15 }}>{n.notes}</Text>
              </View>
            ))}
          </ScrollView>
          <Button title="Got it" onPress={close} />
        </View>
      </View>
    </Modal>
  );
}

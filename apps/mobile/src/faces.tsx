// Known faces (owners), on the phone: add people with the camera, see and remove their faces.

import * as ImagePicker from "expo-image-picker";
import { useCallback, useEffect, useState } from "react";
import { Alert, Pressable, Text, TextInput, View } from "react-native";
import { message, Section } from "./admin";
import { AuthedImage } from "./images";
import type { Outpost } from "./outpost";
import { Button, C, Chip, ErrorText, P, s } from "./ui";

interface Sample {
  id: string;
  created_at: number;
}
interface KnownFace {
  id: string;
  name: string;
  account_id: string | null;
  samples: Sample[];
}
interface FacesState {
  faces: KnownFace[];
  setup: { ready: boolean; busy: boolean; error: string | null };
  voice_installed: boolean;
}

export function KnownFaces({ api }: { api: Outpost }) {
  const [state, setState] = useState<FacesState | null>(null);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setState(await api.get<FacesState>("/api/faces"));
    } catch (e) {
      setError(message(e));
    }
  }, [api]);
  useEffect(() => {
    void load();
  }, [load]);
  useEffect(() => {
    if (!state?.setup.busy) return;
    const t = setInterval(() => void load(), 3000);
    return () => clearInterval(t);
  }, [state?.setup.busy, load]);

  async function run(key: string, work: () => Promise<unknown>) {
    setBusy(key);
    setError(null);
    try {
      await work();
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }

  async function addPhotos(face: KnownFace, camera: boolean) {
    const r = camera
      ? await ImagePicker.launchCameraAsync({ mediaTypes: ["images"], base64: true, quality: 0.85, cameraType: ImagePicker.CameraType.front })
      : await ImagePicker.launchImageLibraryAsync({ mediaTypes: ["images"], base64: true, quality: 0.85, allowsMultipleSelection: true, selectionLimit: 10 });
    if (r.canceled) return;
    await run(face.id, async () => {
      const said: string[] = [];
      for (const a of r.assets) {
        if (!a.base64) continue;
        try {
          const res = await api.json<{ note: string | null }>("POST", `/api/faces/${face.id}/photos`, {
            image: `data:${a.mimeType ?? "image/jpeg"};base64,${a.base64}`,
          });
          if (res.note) said.push(res.note);
        } catch (e) {
          said.push(message(e));
        }
      }
      setNote((n) => ({ ...n, [face.id]: said.join("\n") }));
    });
  }

  function forget(f: KnownFace) {
    Alert.alert(`Forget ${f.name}?`, "Their name and every face of theirs are removed.", [
      { text: "Cancel", style: "cancel" },
      { text: "Forget", style: "destructive", onPress: () => void run(f.id, () => api.json("DELETE", `/api/faces/${f.id}`)) },
    ]);
  }

  return (
    <Section title="Known faces">
      <P muted>
        People Outpost can name at the door, in routines with Say who it is on. Add 5 to 10 clear photos of each face. Faces stay on your server.
        Only add people who are happy to be recognised.
      </P>
      <ErrorText>{error}</ErrorText>
      {!state ? null : !state.setup.ready ? (
        !state.voice_installed ? (
          <P muted>Install Voice on the PC first (Settings › Engines): it's what finds the faces.</P>
        ) : state.setup.busy ? (
          <P muted>Setting up known faces (about 80 MB)…</P>
        ) : (
          <>
            <P muted>Known faces need a small add-on for Voice (about 80 MB).</P>
            <Button title="Set up known faces" busy={busy === "setup"} onPress={() => void run("setup", () => api.json("POST", "/api/faces/setup"))} />
            {state.setup.error ? <ErrorText>{`The last try didn't work: ${state.setup.error}`}</ErrorText> : null}
          </>
        )
      ) : (
        <>
          <View style={s.row}>
            <TextInput
              value={name}
              onChangeText={setName}
              placeholder="Their name"
              placeholderTextColor={C.muted}
              maxLength={60}
              accessibilityLabel="Their name"
              style={{ flex: 1, color: C.ink, borderWidth: 1, borderColor: C.line, borderRadius: 10, paddingHorizontal: 12, minHeight: 44, fontSize: 16 }}
            />
            <Button
              title="Add"
              busy={busy === "new"}
              onPress={() =>
                name.trim() && void run("new", () => api.json("POST", "/api/faces", { name: name.trim() }).then(() => setName("")))
              }
            />
          </View>
          {state.faces.map((f) => (
            <View key={f.id} style={[s.card, { gap: 8 }]}>
              <Text style={{ color: C.ink, fontWeight: "600", fontSize: 16 }}>{f.name}</Text>
              <View style={[s.row, { flexWrap: "wrap", gap: 6 }]}>
                {f.samples.map((x) => (
                  <Pressable
                    key={x.id}
                    accessibilityLabel="Remove this face"
                    onLongPress={() =>
                      Alert.alert("Remove this face?", undefined, [
                        { text: "Cancel", style: "cancel" },
                        { text: "Remove", style: "destructive", onPress: () => void run(f.id, () => api.json("DELETE", `/api/faces/samples/${x.id}`)) },
                      ])
                    }
                  >
                    <AuthedImage api={api} path={`/api/faces/samples/${x.id}`} style={{ width: 56, height: 56, borderRadius: 8 }} />
                  </Pressable>
                ))}
              </View>
              <Text style={{ color: f.samples.length < 5 ? C.amber : C.muted, fontSize: 13 }}>
                {f.samples.length === 0
                  ? "No photos yet."
                  : f.samples.length < 5
                    ? `${f.samples.length} so far: a few more makes matching reliable. Long-press a face to remove it.`
                    : `${f.samples.length} faces. Long-press one to remove it.`}
              </Text>
              {note[f.id] ? <Text style={{ color: C.amber, fontSize: 13 }}>{note[f.id]}</Text> : null}
              <View style={[s.row, { flexWrap: "wrap" }]}>
                <Chip label={busy === f.id ? "Adding…" : "Take a photo"} onPress={() => void addPhotos(f, true)} />
                <Chip label="From photos" onPress={() => void addPhotos(f, false)} />
                <Chip label="Forget" color={C.danger} onPress={() => forget(f)} />
              </View>
            </View>
          ))}
        </>
      )}
    </Section>
  );
}

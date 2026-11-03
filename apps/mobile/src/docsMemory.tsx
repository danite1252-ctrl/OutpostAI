// Documents and memory on the phone. Documents: the ones you've given Outpost to read (add
// more by sharing a file to Outpost from any app), and which a chat uses. Memory: what your
// personas remember about you, learned from chats or added by you.

import { useCallback, useEffect, useState } from "react";
import { Alert, Modal, ScrollView, Text, View } from "react-native";
import { ListState, message, Screen, Section, Toggle } from "./admin";
import type { Outpost } from "./outpost";
import { Button, C, Chip, ErrorText, Field, P, s } from "./ui";

interface Doc {
  id: string;
  name: string;
  bytes: number;
  status: "reading" | "ready" | "failed";
  error: string | null;
  pages: number | null;
}
interface Memory {
  id: string;
  persona_id: string | null;
  text: string;
  source: "manual" | "auto";
  created_at: number;
}
interface Persona {
  id: string;
  name: string;
  owner_name?: string | null;
}

const size = (b: number) => (b >= 1e6 ? `${(b / 1e6).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1e3))} KB`);

export function DocsMemoryScreen({ api, onClose }: { api: Outpost; onClose: () => void }) {
  const [docs, setDocs] = useState<Doc[] | null>(null);
  const [memory, setMemory] = useState<Memory[] | null>(null);
  const [auto, setAuto] = useState<boolean | null>(null);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [text, setText] = useState("");
  const [scope, setScope] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showAll, setShowAll] = useState(false);

  const reload = useCallback(async () => {
    try {
      const [d, m, st, p] = await Promise.all([
        api.get<Doc[]>("/api/documents"),
        api.get<Memory[]>("/api/memory"),
        api.get<{ auto: boolean }>("/api/memory/settings"),
        api.get<Persona[]>("/api/personas"),
      ]);
      setDocs(d);
      setMemory(m);
      setAuto(st.auto);
      setPersonas(p.filter((x) => !x.owner_name));
    } catch (e) {
      setError(message(e));
    }
  }, [api]);
  useEffect(() => {
    void reload();
  }, [reload]);
  // Documents still being read: look again shortly.
  useEffect(() => {
    if (!docs?.some((d) => d.status === "reading")) return;
    const t = setTimeout(() => void reload(), 3000);
    return () => clearTimeout(t);
  }, [docs, reload]);

  async function run(work: () => Promise<unknown>) {
    setError(null);
    try {
      await work();
      await reload();
    } catch (e) {
      setError(message(e));
    }
  }
  function removeDoc(d: Doc) {
    Alert.alert(`Delete ${d.name}?`, "It leaves every chat and persona that uses it.", [
      { text: "Keep", style: "cancel" },
      { text: "Delete", style: "destructive", onPress: () => void run(() => api.json("DELETE", `/api/documents/${d.id}`)) },
    ]);
  }
  function clearLearned() {
    Alert.alert("Forget everything learned from chats?", "Things you added yourself stay.", [
      { text: "Keep", style: "cancel" },
      { text: "Forget", style: "destructive", onPress: () => void run(() => api.json("DELETE", "/api/memory/auto")) },
    ]);
  }
  const add = () =>
    run(async () => {
      await api.json("POST", "/api/memory", { text: text.trim(), persona_id: scope });
      setText("");
    });

  const personaName = (id: string | null) => (id ? (personas.find((p) => p.id === id)?.name ?? "one persona") : "all personas");
  const shownMemory = showAll ? (memory ?? []) : (memory ?? []).slice(0, 30);
  return (
    <Screen title="Documents and memory" onClose={onClose}>
      <ErrorText>{error}</ErrorText>
      <Section title="Documents">
        <P muted>
          To add one, share a file to Outpost from any app (Files, Drive, email: Share › Outpost). PDFs, Word files, text and
          spreadsheets as CSV. Then pick which chats or personas use it.
        </P>
        <ListState loading={docs === null} error={null} empty={docs?.length === 0} emptyText="No documents yet." />
        {(docs ?? []).map((d) => (
          <View key={d.id} style={[s.card, s.row]}>
            <View style={{ flex: 1 }}>
              <Text style={{ color: C.ink }} numberOfLines={1}>
                {d.name}
              </Text>
              <Text style={{ color: d.status === "failed" ? C.danger : C.muted, fontSize: 13 }}>
                {d.status === "reading" ? "Reading it…" : d.status === "failed" ? (d.error ?? "Couldn't read it.") : `${size(d.bytes)}${d.pages ? ` · ${d.pages} pages` : ""}`}
              </Text>
            </View>
            <Chip label="Delete" color={C.danger} onPress={() => removeDoc(d)} />
          </View>
        ))}
      </Section>
      <Section title="Memory">
        {auto !== null && (
          <Toggle
            title="Learn from chats"
            hint="Personas with memory on pick up facts you mention and use them later, when they're relevant."
            on={auto}
            onChange={(on) => void run(() => api.json("PUT", "/api/memory/settings", { auto: on }))}
          />
        )}
        <Field label="Add something to remember" value={text} onChangeText={setText} placeholder="e.g. I'm vegetarian" />
        <View style={[s.row, { flexWrap: "wrap" }]}>
          <Chip label="All personas" active={scope === null} onPress={() => setScope(null)} />
          {personas.map((p) => (
            <Chip key={p.id} label={`Only ${p.name}`} active={scope === p.id} onPress={() => setScope(p.id)} />
          ))}
        </View>
        <Button kind="ghost" title="Remember it" disabled={!text.trim()} onPress={() => void add()} />
        <ListState loading={memory === null} error={null} empty={memory?.length === 0} emptyText="Nothing remembered yet." />
        {shownMemory.map((m) => (
          <View key={m.id} style={[s.card, s.row]}>
            <View style={{ flex: 1 }}>
              <Text style={{ color: C.ink }}>{m.text}</Text>
              <Text style={{ color: C.muted, fontSize: 12 }}>
                {m.source === "auto" ? "Learned" : "Added by you"} · {personaName(m.persona_id)}
              </Text>
            </View>
            <Chip label="Forget" onPress={() => void run(() => api.json("DELETE", `/api/memory/${m.id}`))} />
          </View>
        ))}
        {(memory?.length ?? 0) > 30 && !showAll && <Chip label={`Show all ${memory!.length}`} onPress={() => setShowAll(true)} />}
        {memory?.some((m) => m.source === "auto") && <Button kind="danger" title="Forget everything learned" onPress={clearLearned} />}
      </Section>
    </Screen>
  );
}

/** Which documents this chat can use (the persona's always come along). */
export function ChatDocsSheet({ api, chatId, onClose }: { api: Outpost; chatId: string; onClose: () => void }) {
  const [docs, setDocs] = useState<Doc[]>([]);
  const [on, setOn] = useState<string[]>([]);
  const [fromPersona, setFromPersona] = useState<Doc[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.get<Doc[]>("/api/documents").then(setDocs, (e) => setError(message(e)));
    api.get<{ documents: Doc[]; persona_documents: Doc[] }>(`/api/chats/${chatId}/documents`).then(
      (r) => {
        setOn(r.documents.map((d) => d.id));
        setFromPersona(r.persona_documents);
      },
      () => undefined,
    );
  }, [api, chatId]);
  async function set(ids: string[]) {
    setOn(ids);
    try {
      await api.json("PUT", `/api/chats/${chatId}/documents`, { document_ids: ids });
    } catch (e) {
      setError(message(e));
    }
  }
  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "flex-end" }}>
        <View style={[s.card, { maxHeight: "85%", borderBottomLeftRadius: 0, borderBottomRightRadius: 0 }]}>
          <ScrollView contentContainerStyle={{ gap: 12 }}>
            <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>Documents for this chat</Text>
            <P muted>The chat can look things up in these. Add more by sharing a file to Outpost from any app.</P>
            {docs.length === 0 && <P muted>No documents yet.</P>}
            <View style={[s.row, { flexWrap: "wrap" }]}>
              {docs
                .filter((d) => d.status === "ready")
                .map((d) => {
                  const active = on.includes(d.id);
                  return <Chip key={d.id} label={d.name} active={active} onPress={() => void set(active ? on.filter((x) => x !== d.id) : [...on, d.id])} />;
                })}
            </View>
            {fromPersona.length > 0 && <P muted>Also from the persona: {fromPersona.map((d) => d.name).join(", ")}.</P>}
            <ErrorText>{error}</ErrorText>
            <Button title="Done" onPress={onClose} />
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

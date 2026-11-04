// Personas on the phone: make and change them (prompt, model, tools, documents, voice,
// sharing), send someone a copy, or make your own copy of one shared with you.

import { useCallback, useEffect, useState } from "react";
import { Alert, Pressable, Text, View } from "react-native";
import { ListState, message, Screen, Section, Toggle } from "./admin";
import { chatModels, type ChatModel } from "./models";
import type { Outpost } from "./outpost";
import type { ServerTool } from "./toolLibrary";
import { Avatar, Button, C, Chip, ErrorText, Field, P, s } from "./ui";

export interface FullPersona {
  id: string;
  slug: string;
  name: string;
  avatar_color: string;
  system_prompt: string;
  model: string | null;
  temperature: number | null;
  think: boolean;
  use_memory: boolean;
  voice: string | null;
  speak_replies: boolean;
  shared: boolean;
  /** Someone else's, shared with me (read-only). */
  owner_name?: string | null;
}

interface Voice {
  id: string;
  name: string;
  language: string;
  gender: string;
}
interface VoiceStatus {
  ready: boolean;
  voices: Voice[];
  default_voice: string;
}
interface Doc {
  id: string;
  name: string;
  status: string;
}
interface Person {
  id: string;
  name: string;
  avatar_color: string;
}

export function PersonasScreen({ api, onClose }: { api: Outpost; onClose: () => void }) {
  const [personas, setPersonas] = useState<FullPersona[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<FullPersona | null>(null);
  const [picking, setPicking] = useState(false);
  const reload = useCallback(
    () => api.get<FullPersona[]>("/api/personas").then(setPersonas, (e) => setError(message(e))),
    [api],
  );
  useEffect(() => {
    void reload();
  }, [reload]);

  async function create() {
    try {
      const p = await api.json<FullPersona>("POST", "/api/personas", { name: "New persona", system_prompt: "" });
      await reload();
      setOpen(p);
    } catch (e) {
      setError(message(e));
    }
  }

  if (picking) {
    return (
      <TemplatesScreen
        api={api}
        onAdded={(p) => {
          setPicking(false);
          void reload();
          setOpen(p);
        }}
        onClose={() => setPicking(false)}
      />
    );
  }
  if (open) {
    return (
      <PersonaEditor
        api={api}
        persona={open}
        onClose={() => {
          setOpen(null);
          void reload();
        }}
        onOpen={setOpen}
      />
    );
  }
  const mine = (personas ?? []).filter((p) => !p.owner_name);
  const shared = (personas ?? []).filter((p) => p.owner_name);
  return (
    <Screen title="Personas" onClose={onClose}>
      <P muted>A persona is who you chat with: its instructions, model, tools, documents and voice.</P>
      <Button title="New persona" onPress={() => void create()} />
      <Button kind="ghost" title="Ready-made personas" onPress={() => setPicking(true)} />
      <ListState loading={personas === null} error={error} empty={personas?.length === 0} emptyText="No personas yet." />
      {mine.map((p) => (
        <PersonaRow key={p.id} p={p} onPress={() => setOpen(p)} />
      ))}
      {shared.length > 0 && (
        <Section title="Shared with you">
          {shared.map((p) => (
            <PersonaRow key={p.id} p={p} onPress={() => setOpen(p)} />
          ))}
        </Section>
      )}
    </Screen>
  );
}

function PersonaRow({ p, onPress }: { p: FullPersona; onPress: () => void }) {
  return (
    <Pressable accessibilityRole="button" onPress={onPress} style={[s.card, s.row]}>
      <Avatar name={p.name} color={p.avatar_color} size={36} />
      <View style={{ flex: 1 }}>
        <Text style={{ color: C.ink, fontWeight: "600" }}>{p.name}</Text>
        <Text style={{ color: C.muted, fontSize: 13 }} numberOfLines={1}>
          {p.owner_name ? `Shared by ${p.owner_name}` : p.shared ? "Shared with everyone" : "Yours"}
          {p.model ? ` · ${p.model}` : ""}
        </Text>
      </View>
    </Pressable>
  );
}

function PersonaEditor({
  api,
  persona,
  onClose,
  onOpen,
}: {
  api: Outpost;
  persona: FullPersona;
  onClose: () => void;
  onOpen: (p: FullPersona) => void;
}) {
  const readOnly = !!persona.owner_name;
  const [name, setName] = useState(persona.name);
  const [prompt, setPrompt] = useState(persona.system_prompt);
  const [model, setModel] = useState(persona.model ?? "");
  const [temperature, setTemperature] = useState<number | null>(persona.temperature);
  const [think, setThink] = useState(persona.think);
  const [useMemory, setUseMemory] = useState(persona.use_memory);
  const [voice, setVoice] = useState(persona.voice ?? "");
  const [speak, setSpeak] = useState(persona.speak_replies);
  const [shared, setShared] = useState(persona.shared);
  const [models, setModels] = useState<ChatModel[]>([]);
  const [voices, setVoices] = useState<VoiceStatus | null>(null);
  const [tools, setTools] = useState<ServerTool[]>([]);
  const [toolIds, setToolIds] = useState<string[]>([]);
  const [docs, setDocs] = useState<Doc[]>([]);
  const [docIds, setDocIds] = useState<string[]>([]);
  const [showAllModels, setShowAllModels] = useState(false);
  const [sending, setSending] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    chatModels(api).then(setModels, () => undefined);
    api.get<VoiceStatus>("/api/voice").then(setVoices, () => undefined);
    if (readOnly) return;
    api.get<ServerTool[]>("/api/tools").then(setTools, () => undefined);
    api.get<string[]>(`/api/personas/${persona.id}/tools`).then(setToolIds, () => undefined);
    api.get<Doc[]>("/api/documents").then(setDocs, () => undefined);
    api.get<Doc[]>(`/api/personas/${persona.id}/documents`).then((d) => setDocIds(d.map((x) => x.id)), () => undefined);
  }, [api, persona.id, readOnly]);

  const thinks = models.find((m) => m.model === model)?.badges.includes("thinking") ?? true;

  async function save() {
    setBusy(true);
    setError(null);
    setSaved(null);
    try {
      await api.json("PATCH", `/api/personas/${persona.id}`, {
        name: name.trim() || persona.name,
        system_prompt: prompt,
        model,
        ...(temperature !== null ? { temperature } : {}),
        think,
        use_memory: useMemory,
        voice,
        speak_replies: speak,
        shared,
      });
      await api.json("PUT", `/api/personas/${persona.id}/tools`, { ids: toolIds });
      await api.json("PUT", `/api/personas/${persona.id}/documents`, { document_ids: docIds });
      setSaved("Saved.");
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  function remove() {
    Alert.alert(`Delete ${persona.name}?`, "Its chats are kept.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: () => void api.json("DELETE", `/api/personas/${persona.id}`).then(onClose, (e) => setError(message(e))),
      },
    ]);
  }

  async function copy() {
    try {
      const p = await api.json<FullPersona>("POST", `/api/personas/${persona.id}/copy`);
      onOpen(p);
    } catch (e) {
      setError(message(e));
    }
  }

  if (sending) return <SendCopy api={api} persona={persona} onClose={() => setSending(false)} />;

  if (readOnly) {
    return (
      <Screen title={persona.name} onClose={onClose} back="‹ Personas">
        <View style={s.row}>
          <Avatar name={persona.name} color={persona.avatar_color} size={48} />
          <P muted>Shared by {persona.owner_name}. Only they can change it; make your own copy to change yours.</P>
        </View>
        <Section title="Instructions">
          <P>{persona.system_prompt || "(none)"}</P>
        </Section>
        <P muted>
          Model: {persona.model ?? "Outpost's default"}
          {persona.voice ? ` · Voice: ${persona.voice}` : ""}
        </P>
        <Button title="Make my own copy" onPress={() => void copy()} />
        <ErrorText>{error}</ErrorText>
      </Screen>
    );
  }

  const shownModels = showAllModels ? models : models.slice(0, 8);
  const languages = [...new Set((voices?.voices ?? []).map((v) => v.language))];
  return (
    <Screen title={name || "Persona"} onClose={onClose} back="‹ Personas">
      <Field label="Name" value={name} onChangeText={setName} />
      <Field
        label="Instructions (how it behaves)"
        value={prompt}
        onChangeText={setPrompt}
        multiline
        textAlignVertical="top"
        style={{ minHeight: 140, paddingTop: 10 }}
        placeholder="You are a friendly assistant who…"
      />
      <Section title="Model">
        <View style={[s.row, { flexWrap: "wrap" }]}>
          <Chip label="Outpost's default" active={!model} onPress={() => setModel("")} />
          {shownModels.map((m) => (
            <Chip key={m.model} label={m.lent ? `${m.model} (${m.lent})` : m.model} active={model === m.model} onPress={() => setModel(m.model)} />
          ))}
          {models.length > 8 && <Chip label={showAllModels ? "Fewer" : `All ${models.length}`} onPress={() => setShowAllModels((x) => !x)} />}
        </View>
        {model && !models.some((m) => m.model === model) && models.length > 0 && (
          <Text style={{ color: C.amber, fontSize: 13 }}>{model} isn't downloaded on your Outpost.</Text>
        )}
      </Section>
      <Section title="Tools">
        {tools.length === 0 ? (
          <P muted>No tools yet: add them under You › Tools.</P>
        ) : (
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {tools
              .filter((t) => t.enabled)
              .map((t) => {
                const on = toolIds.includes(t.id);
                return (
                  <Chip key={t.id} label={t.name} active={on} onPress={() => setToolIds(on ? toolIds.filter((x) => x !== t.id) : [...toolIds, t.id])} />
                );
              })}
          </View>
        )}
        <P muted>New chats with this persona start with these. Each chat can change its own from + › Tools.</P>
      </Section>
      <Section title="Documents it knows">
        {docs.length === 0 ? (
          <P muted>No documents yet: add them under You › Documents and memory.</P>
        ) : (
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {docs.map((d) => {
              const on = docIds.includes(d.id);
              return <Chip key={d.id} label={d.name} active={on} onPress={() => setDocIds(on ? docIds.filter((x) => x !== d.id) : [...docIds, d.id])} />;
            })}
          </View>
        )}
      </Section>
      <Section title="Voice">
        {voices && !voices.ready ? (
          <P muted>Voice isn't running on your Outpost (install it on the PC in Settings › Engines).</P>
        ) : (
          <View style={{ gap: 8 }}>
            <View style={[s.row, { flexWrap: "wrap" }]}>
              <Chip label="Default" active={!voice} onPress={() => setVoice("")} />
              {languages.map((lang) => (
                <View key={lang} style={[s.row, { flexWrap: "wrap" }]}>
                  {(voices?.voices ?? [])
                    .filter((v) => v.language === lang)
                    .map((v) => (
                      <Chip key={v.id} label={`${v.name} (${lang})`} active={voice === v.id} onPress={() => setVoice(v.id)} />
                    ))}
                </View>
              ))}
            </View>
            <Toggle title="Read replies aloud" on={speak} onChange={setSpeak} />
          </View>
        )}
      </Section>
      <Section title="Behaviour">
        <Toggle title="Remember things from chats" hint="It learns facts you tell it and uses them later." on={useMemory} onChange={setUseMemory} />
        <Toggle title="Show reasoning" hint={thinks ? "For thinking models." : "This model doesn't think."} on={think} onChange={setThink} />
        <Text style={{ color: C.ink, fontSize: 15 }}>Creativity (temperature)</Text>
        <View style={[s.row, { flexWrap: "wrap" }]}>
          <Chip label="Model default" active={temperature === null} onPress={() => setTemperature(null)} />
          {[0.2, 0.5, 0.7, 1.0].map((t) => (
            <Chip key={t} label={`${t.toFixed(1)}${t === 0.2 ? " precise" : t === 1 ? " creative" : ""}`} active={temperature === t} onPress={() => setTemperature(t)} />
          ))}
        </View>
        <Toggle
          title="Share with everyone"
          hint="They can chat with it (their chats and memories stay theirs) and make their own copy. Only you can change it."
          on={shared}
          onChange={setShared}
        />
      </Section>
      <ErrorText>{error}</ErrorText>
      {saved && <Text style={{ color: C.ok }}>{saved}</Text>}
      <Button title="Save" busy={busy} onPress={() => void save()} />
      <Button kind="ghost" title="Send someone a copy" onPress={() => setSending(true)} />
      <Button kind="danger" title="Delete this persona" onPress={remove} />
    </Screen>
  );
}

function SendCopy({ api, persona, onClose }: { api: Outpost; persona: FullPersona; onClose: () => void }) {
  const [people, setPeople] = useState<Person[] | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.get<Person[]>("/api/people").then(setPeople, (e) => setError(message(e)));
  }, [api]);
  async function send() {
    setBusy(true);
    setError(null);
    try {
      const sent = await api.json<{ account_name: string }[]>("POST", `/api/personas/${persona.id}/send`, { account_ids: chosen });
      setDone(`Sent to ${sent.map((x) => x.account_name).join(", ")}.`);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Screen title={`Send ${persona.name}`} onClose={onClose} back="‹ Back">
      <P muted>Each gets their own copy (its prompt, model, voice and settings) to change as they like. Your memories stay with you. Save first: the copy is made from what's saved.</P>
      <ListState loading={people === null} error={null} empty={people?.length === 0} emptyText="Nobody else uses this Outpost yet." />
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {(people ?? []).map((p) => {
          const on = chosen.includes(p.id);
          return <Chip key={p.id} label={p.name} active={on} onPress={() => setChosen(on ? chosen.filter((x) => x !== p.id) : [...chosen, p.id])} />;
        })}
      </View>
      <ErrorText>{error}</ErrorText>
      {done ? <P>{done}</P> : <Button title="Send" disabled={chosen.length === 0} busy={busy} onPress={() => void send()} />}
    </Screen>
  );
}

interface Template {
  id: string;
  name: string;
  group: string;
  blurb: string;
  avatar_color: string;
  prompt: string;
}

/** Ready-made personas to add (each becomes yours, to change as you like). */
function TemplatesScreen({ api, onAdded, onClose }: { api: Outpost; onAdded: (p: FullPersona) => void; onClose: () => void }) {
  const [list, setList] = useState<Template[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [peek, setPeek] = useState<string | null>(null);
  useEffect(() => {
    api.get<Template[]>("/api/personas/templates").then(setList, (e) => setError(message(e)));
  }, [api]);
  async function add(t: Template) {
    setBusy(t.id);
    setError(null);
    try {
      const r = await api.json<{ persona: FullPersona }>("POST", `/api/personas/templates/${t.id}`);
      onAdded(r.persona);
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }
  const groups = [...new Set((list ?? []).map((t) => t.group))];
  return (
    <Screen title="Ready-made personas" onClose={onClose} back="‹ Personas">
      <P muted>Adding one makes it yours to change. It gets the tools that suit it, if you have them.</P>
      <ListState loading={list === null} error={error} empty={false} emptyText="" />
      {groups.map((g) => (
        <Section key={g} title={g}>
          {(list ?? [])
            .filter((t) => t.group === g)
            .map((t) => (
              <View key={t.id} style={[s.card, { gap: 6 }]}>
                <View style={s.row}>
                  <Avatar name={t.name} color={t.avatar_color} size={32} />
                  <Text style={{ color: C.ink, fontWeight: "600", flex: 1 }}>{t.name}</Text>
                  <Chip label={busy === t.id ? "Adding…" : "Add"} active onPress={() => void add(t)} />
                </View>
                <Text style={{ color: C.soft, fontSize: 14 }}>{t.blurb}</Text>
                <Text style={{ color: C.pick, fontSize: 13 }} onPress={() => setPeek(peek === t.id ? null : t.id)}>
                  {peek === t.id ? "Hide its instructions" : "See its instructions"}
                </Text>
                {peek === t.id && <Text style={{ color: C.muted, fontSize: 13 }}>{t.prompt}</Text>}
              </View>
            ))}
        </Section>
      ))}
    </Screen>
  );
}

// The chat's model, from the phone: pick another (for this chat, or for its persona when
// it's yours), or go back to the persona's. Handy when a model misbehaves while you're out.

import { useEffect, useState } from "react";
import { Modal, ScrollView, Text, View } from "react-native";
import type { Outpost, Persona } from "./outpost";
import { Button, C, Chip, ErrorText, P, s } from "./ui";

export interface ChatModel {
  model: string;
  badges: string[];
  parameter_size?: string | null;
  /** The lent PC it runs on, when it runs there. */
  lent?: string | null;
}

export async function chatModels(api: Outpost): Promise<ChatModel[]> {
  const o = await api.get<{ installed: ChatModel[] }>("/api/models");
  return o.installed.filter((m) => !m.badges.includes("embedding"));
}

const BADGES: Record<string, string> = { tools: "tools", vision: "pictures", thinking: "thinks" };

export function ModelSheet({
  api,
  chatId,
  chatModel,
  persona,
  personaModel,
  onChanged,
  onClose,
}: {
  api: Outpost;
  chatId: string;
  /** This chat's own model (overrides the persona's), if set. */
  chatModel: string | null;
  persona: Persona | null;
  personaModel: string | null;
  onChanged: (change: { chat: string | null; persona: string | null }) => void;
  onClose: () => void;
}) {
  const [models, setModels] = useState<ChatModel[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const mine = !!persona && !persona.owner_name;
  // For a persona that's mine, changing it for every chat is an option.
  const [forPersona, setForPersona] = useState(false);
  useEffect(() => {
    chatModels(api).then(setModels, (e) => setError(e instanceof Error ? e.message : "Couldn't load the models."));
  }, [api]);
  const current = chatModel ?? personaModel;

  async function pick(m: string) {
    setError(null);
    try {
      if (forPersona && persona) {
        await api.json("PATCH", `/api/personas/${persona.id}`, { model: m });
        // The chat follows the persona again.
        if (chatModel) await api.json("PATCH", `/api/chats/${chatId}`, { model: "" });
        onChanged({ chat: null, persona: m });
      } else {
        await api.json("PATCH", `/api/chats/${chatId}`, { model: m });
        onChanged({ chat: m, persona: personaModel });
      }
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't change the model.");
    }
  }

  async function usePersonas() {
    setError(null);
    try {
      await api.json("PATCH", `/api/chats/${chatId}`, { model: "" });
      onChanged({ chat: null, persona: personaModel });
      onClose();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't change the model.");
    }
  }

  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "flex-end" }}>
        <View style={[s.card, { maxHeight: "85%", borderBottomLeftRadius: 0, borderBottomRightRadius: 0 }]}>
          <ScrollView contentContainerStyle={{ gap: 12 }}>
            <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>Model</Text>
            <P muted>
              {current
                ? `${chatModel ? "This chat uses" : `${persona?.name ?? "This chat"} uses`} ${current}.`
                : "This chat uses Outpost's first chat model."}
            </P>
            {mine && (
              <View style={[s.row, { flexWrap: "wrap" }]}>
                <Chip label="This chat only" active={!forPersona} onPress={() => setForPersona(false)} />
                <Chip label={`All ${persona!.name}'s chats`} active={forPersona} onPress={() => setForPersona(true)} />
              </View>
            )}
            {models === null && !error && <P muted>Loading…</P>}
            {models?.length === 0 && <P muted>No chat models yet. Get one in Models on the PC.</P>}
            {models?.map((m) => {
              const on = current?.toLowerCase() === m.model.toLowerCase();
              const notes = [
                m.parameter_size,
                ...m.badges.map((b) => BADGES[b]).filter(Boolean),
                m.lent ? `on ${m.lent}` : null,
              ].filter(Boolean);
              return (
                <View key={m.model} style={[s.row, { alignItems: "center" }]}>
                  <View style={{ flex: 1 }}>
                    <Text style={{ color: C.ink }} numberOfLines={1}>
                      {m.model}
                    </Text>
                    {notes.length > 0 && <Text style={{ color: C.muted, fontSize: 13 }}>{notes.join(" · ")}</Text>}
                  </View>
                  <Chip label={on ? "In use" : "Use"} active={on} onPress={() => void pick(m.model)} />
                </View>
              );
            })}
            {chatModel && persona && (
              <Chip label={`Go back to ${persona.name}'s model${personaModel ? ` (${personaModel})` : ""}`} onPress={() => void usePersonas()} />
            )}
            {error && <ErrorText>{error}</ErrorText>}
            <Button title="Done" onPress={onClose} />
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

// Tools for a chat, on the phone: the tools on your Outpost server (yours and the household's
// shared ones) that this chat uses. Adding tools and signing in to them is done in the
// desktop app (Tool library); once there, they work here too.

import { useCallback, useEffect, useState } from "react";
import { Modal, ScrollView, Text, View } from "react-native";
import type { Outpost } from "./outpost";
import { Button, C, Chip, ErrorText, P, s } from "./ui";

export interface ServerTool {
  id: string;
  name: string;
  enabled: boolean;
  shared: boolean;
  mine: boolean;
  owner_name: string | null;
  signed_in_as: string | null;
}

export function useChatToolNames(api: Outpost, chatId: string) {
  const [names, setNames] = useState<string[]>([]);
  const reload = useCallback(async () => {
    const [all, ids] = await Promise.all([
      api.get<ServerTool[]>("/api/tools").catch(() => [] as ServerTool[]),
      api.get<string[]>(`/api/chats/${chatId}/tools`).catch(() => [] as string[]),
    ]);
    setNames(all.filter((t) => t.enabled && ids.includes(t.id)).map((t) => t.name));
  }, [api, chatId]);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { names, reload };
}

export function ToolsSheet({ api, chatId, onClose }: { api: Outpost; chatId: string; onClose: () => void }) {
  const [tools, setTools] = useState<ServerTool[]>([]);
  const [on, setOn] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.get<ServerTool[]>("/api/tools").then(setTools, () => undefined);
    api.get<string[]>(`/api/chats/${chatId}/tools`).then(setOn, () => undefined);
  }, [api, chatId]);
  async function set(ids: string[] | null) {
    setError(null);
    try {
      if (ids) setOn(await api.json<string[]>("PUT", `/api/chats/${chatId}/tools`, { ids }));
      else {
        await api.json("DELETE", `/api/chats/${chatId}/tools`);
        setOn(await api.get<string[]>(`/api/chats/${chatId}/tools`));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't change that.");
    }
  }
  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "flex-end" }}>
        <View style={[s.card, { maxHeight: "85%", borderBottomLeftRadius: 0, borderBottomRightRadius: 0 }]}>
          <ScrollView contentContainerStyle={{ gap: 12 }}>
            <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>Tools for this chat</Text>
            <P muted>
              Tools on your Outpost: search, Google, Home Assistant and more. The chat's model needs the tools badge. Hands-free asks
              before each one runs.
            </P>
            {tools.length === 0 && <P muted>No tools yet. Add them in the Outpost desktop app (+ › Tools › Tool library).</P>}
            {tools.map((t) => {
              const active = on.includes(t.id);
              return (
                <View key={t.id} style={[s.row, { alignItems: "center" }]}>
                  <View style={{ flex: 1 }}>
                    <Text style={{ color: t.enabled ? C.ink : C.muted }}>{t.name}</Text>
                    <Text style={{ color: C.muted, fontSize: 13 }}>
                      {!t.enabled ? "Switched off" : t.mine ? (t.signed_in_as ?? "Yours") : `Shared by ${t.owner_name ?? "an owner"}`}
                    </Text>
                  </View>
                  <Chip
                    label={active ? "On" : "Off"}
                    active={active}
                    onPress={() => void set(active ? on.filter((x) => x !== t.id) : [...on, t.id])}
                  />
                </View>
              );
            })}
            {tools.length > 0 && <Chip label="Use the persona's tools" onPress={() => void set(null)} />}
            {error && <ErrorText>{error}</ErrorText>}
            <Button title="Done" onPress={onClose} />
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

// Models on the phone: what's downloaded on your Outpost (and on a lent PC), downloads in
// progress with Cancel, and the catalogue scored for the server's graphics card. Owners
// download and remove; everyone else can look.

import { useCallback, useEffect, useState } from "react";
import { Alert, Text, View } from "react-native";
import { ListState, message, Screen, Section, usePin } from "./admin";
import type { Outpost } from "./outpost";
import { Bar, Button, C, Chip, ErrorText, Field, P, s } from "./ui";

interface Installed {
  model: string;
  size_bytes: number;
  parameter_size: string | null;
  quantization: string | null;
  badges: string[];
  here?: boolean;
  lent?: string | null;
}
interface Entry {
  model: string;
  name: string;
  category: "uncensored" | "general" | "coding" | "embedding";
  size_gb: number;
  context_k: number;
  badges: string[];
  description: string;
  fit: "fits" | "partial" | "too_large";
  recommended: boolean;
}
interface Pull {
  model: string;
  status: string;
  percent: number | null;
  done: boolean;
  error: string | null;
  completed: number;
  total: number;
}
interface Overview {
  hardware: { vram_mb: number; ram_mb: number };
  catalog: Entry[];
  installed: Installed[];
  pulls: Pull[];
  ollama_running: boolean;
}

const CATEGORIES: { id: Entry["category"]; title: string }[] = [
  { id: "general", title: "General" },
  { id: "uncensored", title: "Unfiltered" },
  { id: "coding", title: "Coding" },
  { id: "embedding", title: "For memory and search" },
];
const FIT: Record<Entry["fit"], string> = {
  fits: "Fits the card: fast",
  partial: "Partly on the processor: slower",
  too_large: "Too big for the server",
};
const BADGES: Record<string, string> = { tools: "tools", vision: "pictures", thinking: "thinks", code: "code", uncensored: "unfiltered" };
const gb = (bytes: number) => `${(bytes / 1e9).toFixed(1)} GB`;

export function ModelsScreen({ api, isOwner, onClose }: { api: Outpost; isOwner: boolean; onClose: () => void }) {
  const [data, setData] = useState<Overview | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [category, setCategory] = useState<Entry["category"]>("general");
  const [other, setOther] = useState("");
  const { guard, prompt } = usePin();
  const reload = useCallback(
    () =>
      api.get<Overview>("/api/models").then(
        (d) => {
          setData(d);
          setError(null);
        },
        (e) => setError(message(e)),
      ),
    [api],
  );
  const running = (data?.pulls ?? []).filter((p) => !p.done);
  useEffect(() => {
    void reload();
    const t = setInterval(() => void reload(), running.length ? 1500 : 8000);
    return () => clearInterval(t);
  }, [reload, running.length]);

  async function pull(model: string) {
    setError(null);
    try {
      await guard((h) => api.json("POST", "/api/models/pull", { model }, h));
      await reload();
    } catch (e) {
      setError(message(e));
    }
  }
  async function cancel(model: string) {
    try {
      await api.json("POST", "/api/downloads/cancel", { id: `model:${model}` });
      await reload();
    } catch (e) {
      setError(message(e));
    }
  }
  function remove(model: string) {
    Alert.alert(`Remove ${model}?`, "You can download it again later.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Remove",
        style: "destructive",
        onPress: () =>
          void guard((h) => api.json("POST", "/api/models/remove", { model }, h)).then(
            () => void reload(),
            (e) => setError(message(e)),
          ),
      },
    ]);
  }
  function get(e: Entry) {
    if (e.fit === "partial") {
      Alert.alert(`${e.name} is bigger than the server's graphics card`, `It will run partly on the processor, so slower. Download ${e.size_gb} GB anyway?`, [
        { text: "Cancel", style: "cancel" },
        { text: "Download", onPress: () => void pull(e.model) },
      ]);
    } else void pull(e.model);
  }

  const installed = data?.installed ?? [];
  const have = new Set(installed.map((m) => m.model.toLowerCase()));
  const here = installed.filter((m) => m.here !== false);
  const lent = installed.filter((m) => m.here === false || m.lent);
  const failed = (data?.pulls ?? []).filter((p) => p.done && p.error);
  const entries = (data?.catalog ?? []).filter((e) => e.category === category);
  const vram = data ? Math.round(data.hardware.vram_mb / 1024) : null;

  return (
    <Screen title="Models" onClose={onClose}>
      {prompt}
      <P muted>
        Chat models on your Outpost{vram != null ? ` (its graphics card has ${vram} GB)` : ""}.{" "}
        {isOwner ? "Download and remove them here." : "Owners download and remove them."}
      </P>
      {data && !data.ollama_running && <Text style={{ color: C.amber }}>Ollama isn't running on the server right now.</Text>}
      <ListState loading={data === null} error={error} empty={false} emptyText="" />

      {running.length > 0 && (
        <Section title="Downloading">
          {running.map((p) => (
            <View key={p.model} style={[s.card, { gap: 6 }]}>
              <View style={s.row}>
                <Text style={{ color: C.ink, flex: 1 }} numberOfLines={1}>
                  {p.model}
                </Text>
                {isOwner && <Chip label="Cancel" color={C.danger} onPress={() => void cancel(p.model)} />}
              </View>
              <Bar fraction={p.percent != null ? p.percent / 100 : null} />
              <Text style={{ color: C.muted, fontSize: 13 }}>
                {p.total ? `${gb(p.completed)} of ${gb(p.total)}` : p.status}
                {p.percent != null ? ` · ${Math.round(p.percent)}%` : ""}
              </Text>
            </View>
          ))}
        </Section>
      )}
      {failed.map((p) => (
        <ErrorText key={p.model}>{`${p.model}: ${p.error}`}</ErrorText>
      ))}

      <Section title="On your Outpost">
        {here.length === 0 && data && <P muted>No models yet.</P>}
        {here.map((m) => (
          <ModelRow key={m.model} m={m} onRemove={isOwner ? () => remove(m.model) : undefined} />
        ))}
      </Section>
      {lent.length > 0 && (
        <Section title="On a lent PC">
          <P muted>These run on the PC lending its card (downloaded and removed on that PC). While it's off or paused they aren't available.</P>
          {lent.map((m) => (
            <ModelRow key={`lent-${m.model}`} m={m} />
          ))}
        </Section>
      )}

      {isOwner && (
        <Section title="Get more">
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {CATEGORIES.map((c) => (
              <Chip key={c.id} label={c.title} active={category === c.id} onPress={() => setCategory(c.id)} />
            ))}
          </View>
          {entries.map((e) => {
            const got = have.has(e.model.toLowerCase());
            const pulling = running.some((p) => p.model === e.model);
            return (
              <View key={e.model} style={[s.card, { gap: 4 }]}>
                <Text style={{ color: C.ink, fontWeight: "600" }}>
                  {e.name}
                  {e.recommended ? <Text style={{ color: C.ember, fontWeight: "400", fontSize: 12 }}>  Best pick</Text> : null}
                </Text>
                <Text style={{ color: C.soft, fontSize: 14 }}>{e.description}</Text>
                <Text style={{ color: e.fit === "fits" ? C.ok : e.fit === "partial" ? C.amber : C.danger, fontSize: 13 }}>
                  {e.size_gb} GB · {FIT[e.fit]}
                  {e.badges.length ? ` · ${e.badges.map((b) => BADGES[b] ?? b).join(", ")}` : ""}
                </Text>
                {got ? (
                  <Text style={{ color: C.muted, fontSize: 13 }}>Downloaded</Text>
                ) : pulling ? (
                  <Text style={{ color: C.muted, fontSize: 13 }}>Downloading…</Text>
                ) : e.fit !== "too_large" ? (
                  <View style={s.row}>
                    <Chip label={`Download · ${e.size_gb} GB`} active onPress={() => get(e)} />
                  </View>
                ) : null}
              </View>
            );
          })}
          <Field
            label="Or any Ollama model by name"
            placeholder="e.g. qwen3:14b or hf.co/user/model:Q4_K_M"
            value={other}
            onChangeText={setOther}
            autoCapitalize="none"
            autoCorrect={false}
          />
          <Button
            kind="ghost"
            title="Download it"
            disabled={!other.trim()}
            onPress={() => {
              void pull(other.trim());
              setOther("");
            }}
          />
        </Section>
      )}
    </Screen>
  );
}

function ModelRow({ m, onRemove }: { m: Installed; onRemove?: () => void }) {
  const notes = [
    m.parameter_size,
    m.quantization,
    gb(m.size_bytes),
    ...m.badges.map((b) => BADGES[b]).filter(Boolean),
    m.lent ? `runs on ${m.lent}` : null,
  ].filter(Boolean);
  return (
    <View style={[s.card, s.row]}>
      <View style={{ flex: 1 }}>
        <Text style={{ color: C.ink }} numberOfLines={1}>
          {m.model}
        </Text>
        <Text style={{ color: C.muted, fontSize: 13 }}>{notes.join(" · ")}</Text>
      </View>
      {onRemove && <Chip label="Remove" color={C.danger} onPress={onRemove} />}
    </View>
  );
}

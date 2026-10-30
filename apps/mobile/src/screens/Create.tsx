import * as ImagePicker from "expo-image-picker";
import { useCallback, useEffect, useMemo, useState, type ReactNode } from "react";
import { Dimensions, Image, Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { AuthedImage } from "../images";
import type { Media, Outpost, Pack, PackInput, Task } from "../outpost";
import { Button, C, Chip, ErrorText, Field, H1, P, s } from "../ui";

const TASKS: { id: Task; label: string }[] = [
  { id: "picture", label: "Text to image" },
  { id: "character", label: "Characters" },
  { id: "edit", label: "Edit" },
  { id: "video", label: "Video" },
  { id: "animate", label: "Animate" },
];

type Values = Record<string, unknown>;

function defaults(p: Pack): Values {
  const v: Values = {};
  for (const i of p.inputs) v[i.key] = i.default ?? (i.kind === "text" ? "" : null);
  return v;
}

function fitText(p: Pack) {
  if (p.missing_nodes.length) return { text: "Needs a newer ComfyUI", color: C.muted };
  if (p.fit === "too_large") return { text: "Too big for the PC", color: C.danger };
  if (p.fit === "partial") return { text: "Runs slowly", color: C.warn };
  if (p.recommended) return { text: "Best for the PC", color: C.pick };
  return { text: "Fits", color: C.ok };
}

export default function Create({
  api,
  onError,
  incoming,
  onIncomingUsed,
  onQueued,
}: {
  api: Outpost;
  onError: (e: unknown) => void;
  /** A gallery picture to edit or animate. */
  incoming: { dataUri: string; task: "edit" | "animate" } | null;
  onIncomingUsed: () => void;
  onQueued: () => void;
}) {
  const [packs, setPacks] = useState<Pack[]>([]);
  const [loras, setLoras] = useState<string[]>([]);
  // LoRA file -> my trained character.
  const [names, setNames] = useState<Record<string, { name: string; trigger: string; subject: string | null; style: string | null }>>({});
  const [task, setTask] = useState<Task>("picture");
  const [chosen, setChosen] = useState<Partial<Record<Task, string>>>({});
  const [values, setValues] = useState<Values>({});
  const [more, setMore] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  const load = useCallback(() => {
    api.get<Pack[]>("/api/packs").then(setPacks, (e) => {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't load workflows.");
    });
    api.get<{ loras: string[]; characters?: { name: string; trigger_word: string; lora_file: string; subject: string | null; style: string | null }[] }>("/api/loras").then(
      (r) => {
        setLoras(r.loras);
        setNames(Object.fromEntries((r.characters ?? []).map((c) => [c.lora_file, { name: c.name, trigger: c.trigger_word, subject: c.subject, style: c.style }])));
      },
      () => undefined,
    );
  }, [api, onError]);
  useEffect(load, [load]);

  // Only tabs with a workflow that suits the Outpost's graphics card.
  const shownTasks = useMemo(
    () => (packs.length ? TASKS.filter((t) => packs.some((p) => p.task === t.id)) : TASKS),
    [packs],
  );
  useEffect(() => {
    if (shownTasks.length && !shownTasks.some((t) => t.id === task)) setTask(shownTasks[0].id);
  }, [shownTasks, task]);

  const options = useMemo(
    () =>
      packs
        .filter((p) => p.task === task)
        .sort((a, b) => Number(b.recommended) - Number(a.recommended) || b.rank - a.rank),
    [packs, task],
  );
  const pack = options.find((p) => p.id === chosen[task]) ?? options[0] ?? null;

  useEffect(() => {
    if (!pack) return;
    setValues((v) => {
      const next = defaults(pack);
      for (const i of pack.inputs) if ((i.kind === "text" || i.kind === "image") && v[i.key]) next[i.key] = v[i.key];
      return next;
    });
  }, [pack?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // A picture sent from the gallery.
  useEffect(() => {
    if (!incoming) return;
    setTask(incoming.task);
  }, [incoming]);
  useEffect(() => {
    if (!incoming || !pack || pack.task !== incoming.task) return;
    const input = pack.inputs.find((i) => i.kind === "image");
    if (input) setValues((v) => ({ ...v, [input.key]: incoming.dataUri }));
    onIncomingUsed();
  }, [incoming, pack, onIncomingUsed]);

  async function generate() {
    if (!pack) return;
    setBusy(true);
    setError(null);
    setNotice(null);
    try {
      const inputs: Values = {};
      for (const i of pack.inputs) {
        const v = values[i.key];
        if (v !== null && v !== undefined && v !== "") inputs[i.key] = v;
      }
      await api.json("POST", "/api/jobs", { pack: pack.id, inputs });
      setNotice("Added to the queue.");
      onQueued();
    } catch (e) {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't add the job.");
    } finally {
      setBusy(false);
    }
  }

  const usable = pack && pack.models.ready && pack.missing_nodes.length === 0 && pack.fit !== "too_large";
  const missing = pack?.inputs.some((i) => i.required && !values[i.key]) ?? true;
  const basic = pack?.inputs.filter((i) => !i.advanced) ?? [];
  const advanced = pack?.inputs.filter((i) => i.advanced) ?? [];
  // For a character workflow, the prompt's hint names the character picked.
  const loraInput = pack?.inputs.find((i) => i.kind === "lora");
  const character = loraInput ? names[values[loraInput.key] as string] : undefined;
  const promptHelp = !loraInput
    ? undefined
    : character
      ? `Start with ${character.name}'s trigger word and what they are, e.g. "${character.trigger}${
          character.subject ? ` ${character.subject}` : ""
        }, at the beach at sunset, ${character.style ?? "photo"}". In chat: "draw ${character.name} at the beach".`
      : "Pick a character above, then start the prompt with its trigger word.";

  return (
    <ScrollView style={s.screen} contentContainerStyle={s.pad} keyboardShouldPersistTaps="handled">
      <H1>Create</H1>
      <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={{ gap: 8 }}>
        {shownTasks.map((t) => (
          <Chip key={t.id} label={t.label} active={t.id === task} onPress={() => setTask(t.id)} />
        ))}
      </ScrollView>
      {options.length > 1 && (
        <View style={{ gap: 6 }}>
          <Text style={s.label}>Model</Text>
          {options.map((p) => {
            const f = fitText(p);
            const blocked = p.fit === "too_large" || p.missing_nodes.length > 0;
            return (
              <Pressable
                key={p.id}
                accessibilityRole="radio"
                accessibilityState={{ checked: p.id === pack?.id, disabled: blocked }}
                disabled={blocked}
                onPress={() => setChosen((c) => ({ ...c, [task]: p.id }))}
                style={[s.card, { paddingVertical: 10, opacity: blocked ? 0.45 : 1 }, p.id === pack?.id && { borderColor: C.ember }]}
              >
                <Text style={{ color: C.ink, fontWeight: "600" }}>{p.name}</Text>
                <Text style={{ color: f.color, fontSize: 13 }}>
                  {f.text}
                  {p.models.ready ? "" : ` · ${p.models.missing_gb.toFixed(1)} GB to download on the PC`}
                </Text>
              </Pressable>
            );
          })}
        </View>
      )}
      {pack && (
        <>
          <P muted>{pack.description}</P>
          {!pack.models.ready ? (
            <P muted>Its models aren't downloaded yet. Download them in Create on the PC, then come back.</P>
          ) : pack.missing_nodes.length > 0 ? (
            <P muted>This workflow needs a newer ComfyUI. Update it in Settings › Engines on the PC.</P>
          ) : (
            <>
              {basic.map((i) => (
                <Input key={i.key} api={api} input={i} value={values[i.key]} loras={loras} names={names} help={i.key === "prompt" ? promptHelp : undefined} onChange={(v) => setValues((s) => ({ ...s, [i.key]: v }))} />
              ))}
              {advanced.length > 0 && (
                <Pressable accessibilityRole="button" onPress={() => setMore((m) => !m)}>
                  <Text style={{ color: C.muted }}>{more ? "▾ Fewer settings" : "▸ More settings"}</Text>
                </Pressable>
              )}
              {more &&
                advanced.map((i) => (
                  <Input key={i.key} api={api} input={i} value={values[i.key]} loras={loras} names={names} help={i.key === "prompt" ? promptHelp : undefined} onChange={(v) => setValues((s) => ({ ...s, [i.key]: v }))} />
                ))}
              <Button title="Generate" busy={busy} disabled={!usable || missing} onPress={() => void generate()} />
            </>
          )}
        </>
      )}
      <ErrorText>{error}</ErrorText>
      {notice && <P style={{ color: C.ok }}>{notice}</P>}
    </ScrollView>
  );
}

function sizeLabel(o: string) {
  const [w, h] = o.split("x").map(Number);
  if (!w || !h) return o;
  return `${w === h ? "Square" : w > h ? "Landscape" : "Portrait"} ${w}×${h}`;
}

function Input({
  api,
  input: i,
  value,
  loras,
  names,
  help: helpText,
  onChange,
}: {
  api: Outpost;
  input: PackInput;
  value: unknown;
  loras: string[];
  names: Record<string, { name: string; trigger: string; subject: string | null; style: string | null }>;
  /** Replaces the input's own hint. */
  help?: string;
  onChange: (v: unknown) => void;
}) {
  const label = `${i.label}${i.required ? " *" : ""}`;
  const hint = helpText ?? i.help;
  const help = hint ? <Text style={{ color: C.muted, fontSize: 13 }}>{hint}</Text> : null;
  switch (i.kind) {
    case "text":
      return (
        <View style={{ gap: 6 }}>
          <Field label={label} value={(value as string) ?? ""} onChangeText={onChange} multiline maxLength={4000} style={{ minHeight: 96, textAlignVertical: "top", paddingTop: 12 }} />
          {help}
        </View>
      );
    case "size":
    case "choice":
      return (
        <View style={{ gap: 6 }}>
          <Text style={s.label}>{label}</Text>
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {i.options.map((o) => (
              <Chip key={o} label={i.kind === "size" ? sizeLabel(o) : o} active={value === o} onPress={() => onChange(o)} />
            ))}
          </View>
          {help}
        </View>
      );
    case "integer":
    case "number": {
      const n = Number(value ?? i.default ?? 0);
      const step = i.step ?? (i.kind === "integer" ? 1 : 0.05);
      const clamp = (x: number) => Math.min(i.max ?? Infinity, Math.max(i.min ?? -Infinity, Number(x.toFixed(2))));
      return (
        <View style={{ gap: 6 }}>
          <Text style={s.label}>{label}</Text>
          <View style={s.row}>
            <Chip label="−" onPress={() => onChange(clamp(n - step))} />
            <Text style={{ color: C.ink, fontSize: 18, minWidth: 56, textAlign: "center" }}>{n}</Text>
            <Chip label="+" onPress={() => onChange(clamp(n + step))} />
          </View>
          {help}
        </View>
      );
    }
    case "seed":
      return (
        <View style={{ gap: 6 }}>
          <Field
            label={label}
            placeholder="Random"
            keyboardType="number-pad"
            value={value == null ? "" : String(value)}
            onChangeText={(t) => {
              const d = t.replace(/\D/g, "").slice(0, 15);
              onChange(d ? Number(d) : null);
            }}
          />
          {help}
        </View>
      );
    case "lora":
      return (
        <View style={{ gap: 6 }}>
          <Text style={s.label}>{label}</Text>
          <View style={[s.row, { flexWrap: "wrap" }]}>
            {!i.required && <Chip label="None" active={!value} onPress={() => onChange(null)} />}
            {loras.map((l) => (
              <Chip key={l} label={names[l]?.name ?? l.split("/").pop()!.replace(/\.safetensors$/i, "")} active={value === l} onPress={() => onChange(l)} />
            ))}
          </View>
          {loras.length === 0 && <P muted>No LoRAs yet: copy them into the LoRA folder on the PC.</P>}
          {help}
        </View>
      );
    case "image":
      return <ImageInput api={api} label={label} help={help} value={value as string | null} onChange={onChange} />;
  }
}

function ImageInput({
  api,
  label,
  help,
  value,
  onChange,
}: {
  api: Outpost;
  label: string;
  help: ReactNode;
  value: string | null;
  onChange: (v: unknown) => void;
}) {
  const [choosing, setChoosing] = useState(false);
  const [loading, setLoading] = useState(false);
  async function pick(camera: boolean) {
    const opts: ImagePicker.ImagePickerOptions = { mediaTypes: ["images"], base64: true, quality: 0.85 };
    const r = camera ? await ImagePicker.launchCameraAsync(opts) : await ImagePicker.launchImageLibraryAsync(opts);
    if (r.canceled || !r.assets[0]?.base64) return;
    onChange(`data:${r.assets[0].mimeType ?? "image/jpeg"};base64,${r.assets[0].base64}`);
  }
  return (
    <View style={{ gap: 6 }}>
      <Text style={s.label}>{label}</Text>
      {value ? <Image source={{ uri: value }} style={{ width: "100%", height: 220, borderRadius: 12 }} resizeMode="contain" /> : null}
      <View style={[s.row, { flexWrap: "wrap" }]}>
        <Button kind="ghost" title="From gallery" busy={loading} onPress={() => setChoosing(true)} style={{ flexGrow: 1 }} />
        <Button kind="ghost" title="Phone photos" onPress={() => void pick(false)} style={{ flexGrow: 1 }} />
        <Button kind="ghost" title="Camera" onPress={() => void pick(true)} />
        {value ? <Button kind="ghost" title="Remove" onPress={() => onChange(null)} /> : null}
      </View>
      {help}
      {choosing && (
        <GalleryPicker
          api={api}
          onClose={() => setChoosing(false)}
          onPick={(m) => {
            setChoosing(false);
            setLoading(true);
            api
              .image(`/api/media/${m.id}`)
              .then((uri) => onChange(uri))
              .catch(() => undefined)
              .finally(() => setLoading(false));
          }}
        />
      )}
    </View>
  );
}

/** Pick one of your Outpost pictures (yours or shared) as an input. */
export function GalleryPicker({ api, onPick, onClose }: { api: Outpost; onPick: (m: Media) => void; onClose: () => void }) {
  const [tab, setTab] = useState<"mine" | "shared">("mine");
  const [items, setItems] = useState<Media[] | null>(null);
  useEffect(() => {
    setItems(null);
    api.get<Media[]>(tab === "mine" ? "/api/gallery?limit=60" : "/api/gallery/shared?limit=60").then(
      (list) => setItems(list.filter((m) => m.kind === "image")),
      () => setItems([]),
    );
  }, [api, tab]);
  const size = Math.floor((Dimensions.get("window").width - 16 * 2 - 8 * 2) / 3);
  const insets = useSafeAreaInsets();
  return (
    <Modal visible animationType="slide" onRequestClose={onClose}>
      <ScrollView
        style={{ backgroundColor: C.night }}
        contentContainerStyle={[s.pad, { paddingTop: insets.top + 16, paddingBottom: insets.bottom + 16 }]}
      >
        <H1>Choose a picture</H1>
        <View style={s.row}>
          <Chip label="Mine" active={tab === "mine"} onPress={() => setTab("mine")} />
          <Chip label="Shared" active={tab === "shared"} onPress={() => setTab("shared")} />
        </View>
        {items === null ? (
          <P muted>Loading…</P>
        ) : items.length === 0 ? (
          <P muted>No pictures here yet.</P>
        ) : (
          <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
            {items.map((m) => (
              <Pressable key={m.id} accessibilityRole="button" accessibilityLabel={m.prompt ?? "Picture"} onPress={() => onPick(m)}>
                <AuthedImage api={api} path={`/api/media/${m.id}`} style={{ width: size, height: size, borderRadius: 10 }} />
              </Pressable>
            ))}
          </View>
        )}
        <Button kind="ghost" title="Cancel" onPress={onClose} />
      </ScrollView>
    </Modal>
  );
}

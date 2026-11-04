import { clock } from "../outpost";
import * as ImagePicker from "expo-image-picker";
import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Alert, BackHandler, Image, Keyboard, Modal, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { MicIcon, PlusIcon, SendIcon, StopIcon } from "../icons";
import { AuthedImage } from "../images";
import type { ChatStart } from "./Share";
import { GalleryPicker } from "./Create";
import HandsFree from "./HandsFree";
import { hear, onSaying, say, stopSaying, type Listening } from "../voice";
import type { Chat as ChatT, Message, Outpost, Persona } from "../outpost";
import { Avatar, Button, C, Chip, ErrorText, H1, P, s } from "../ui";
import { FileActions, FilesSheet, useChatFiles, type FileAction } from "../chatfiles";
import { ToolsSheet, useChatToolNames } from "../tools";
import { ModelSheet } from "../models";
import { ChatDocsSheet } from "../docsMemory";

const MAX_IMAGES = 4;

export default function Chat({
  api,
  onError,
  incoming,
  onIncomingUsed,
}: {
  api: Outpost;
  onError: (e: unknown) => void;
  /** Something shared from another app, to start a new chat with. */
  incoming?: ChatStart | null;
  onIncomingUsed?: () => void;
}) {
  const [chats, setChats] = useState<ChatT[]>([]);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [open, setOpen] = useState<ChatT | null>(null);
  const [error, setError] = useState<string | null>(null);
  // What the new chat starts with (from a share), until it's opened.
  const [starting, setStarting] = useState<ChatStart | null>(null);
  useEffect(() => {
    if (incoming) {
      setOpen(null);
      setStarting(incoming);
      onIncomingUsed?.();
    }
  }, [incoming, onIncomingUsed]);

  const load = useCallback(async () => {
    try {
      const [c, p] = await Promise.all([api.get<ChatT[]>("/api/chats"), api.get<Persona[]>("/api/personas")]);
      setChats(c);
      setPersonas(p);
    } catch (e) {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't load chats.");
    }
  }, [api, onError]);
  useEffect(() => {
    void load();
  }, [load]);

  const [opening, setOpening] = useState<ChatStart | null>(null);
  async function start(persona?: Persona) {
    try {
      const chat = await api.json<ChatT>("POST", "/api/chats", { persona_id: persona?.id ?? personas[0]?.id ?? null });
      if (starting?.documentIds.length) {
        await api.json("PUT", `/api/chats/${chat.id}/documents`, { document_ids: starting.documentIds });
      }
      setChats((c) => [chat, ...c]);
      setOpening(starting);
      setStarting(null);
      setOpen(chat);
    } catch (e) {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't start a chat.");
    }
  }

  // Long-press a chat: rename it, or delete it.
  const [renaming, setRenaming] = useState<ChatT | null>(null);
  const [newTitle, setNewTitle] = useState("");
  const chatMenu = (c: ChatT) =>
    Alert.alert(c.title, undefined, [
      {
        text: "Rename",
        onPress: () => {
          setNewTitle(c.title);
          setRenaming(c);
        },
      },
      { text: "Delete", style: "destructive", onPress: () => removeChat(c) },
      { text: "Cancel", style: "cancel" },
    ]);
  async function rename() {
    const c = renaming;
    const t = newTitle.trim();
    setRenaming(null);
    if (!c || !t || t === c.title) return;
    try {
      const chat = await api.json<ChatT>("PATCH", `/api/chats/${c.id}`, { title: t.slice(0, 120) });
      setChats((list) => list.map((x) => (x.id === chat.id ? chat : x)));
    } catch (e) {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't rename that chat.");
    }
  }

  // Delete a chat (with its pictures, and what the persona learned from it).
  const removeChat = (c: ChatT) =>
    Alert.alert("Delete this chat?", `"${c.title}" goes, with its pictures and anything the persona learned from it.`, [
      { text: "Cancel", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: () =>
          void api.json("DELETE", `/api/chats/${c.id}`).then(
            () => setChats((list) => list.filter((x) => x.id !== c.id)),
            (e) => {
              onError(e);
              setError(e instanceof Error ? e.message : "Couldn't delete that chat.");
            },
          ),
      },
    ]);

  if (open) {
    return (
      <Conversation
        api={api}
        chat={open}
        persona={personas.find((p) => p.id === open.persona_id) ?? null}
        initial={opening}
        onBack={() => {
          setOpening(null);
          setOpen(null);
          void load();
        }}
        onError={onError}
      />
    );
  }

  return (
    <ScrollView style={s.screen} contentContainerStyle={s.pad}>
      <H1>Chat</H1>
      {starting && (
        <View style={[s.card, { borderColor: C.ember, gap: 8 }]}>
          <Text style={{ color: C.ink, fontWeight: "600" }}>
            {starting.documentIds.length
              ? `Ask about ${starting.documentIds.length === 1 ? "your document" : `${starting.documentIds.length} documents`}: pick who to ask`
              : "Pick who to ask about what you shared"}
          </Text>
          <Pressable accessibilityRole="button" onPress={() => setStarting(null)}>
            <Text style={{ color: C.muted }}>Cancel</Text>
          </Pressable>
        </View>
      )}
      <Text style={s.label}>New chat with</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {personas.map((p) => (
          <Chip key={p.id} label={p.name} onPress={() => void start(p)} />
        ))}
        {personas.length === 0 && <Button title="New chat" onPress={() => void start()} />}
      </View>
      <ErrorText>{error}</ErrorText>
      {chats.map((c) => {
        const p = personas.find((x) => x.id === c.persona_id);
        return (
          <Pressable
            key={c.id}
            accessibilityRole="button"
            accessibilityHint="Long-press to rename or delete"
            onPress={() => setOpen(c)}
            onLongPress={() => chatMenu(c)}
            delayLongPress={450}
            style={({ pressed }) => [s.card, s.row, pressed && { backgroundColor: C.raised }]}
          >
            <Avatar name={p?.name ?? "AI"} color={p?.avatar_color ?? C.amber} />
            <View style={{ flex: 1 }}>
              <Text numberOfLines={1} style={{ color: C.ink, fontSize: 16 }}>
                {c.title}
              </Text>
              <Text style={{ color: C.muted, fontSize: 13 }}>{p?.name ?? "No persona"}</Text>
            </View>
          </Pressable>
        );
      })}
      {chats.length === 0 ? <P muted>No chats yet. Everything stays on your PC.</P> : <P muted>Long-press a chat to rename or delete it.</P>}
      <Modal visible={!!renaming} transparent animationType="fade" onRequestClose={() => setRenaming(null)}>
        <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "center", padding: 24 }}>
          <View style={[s.card, { gap: 12 }]}>
            <Text style={{ color: C.ink, fontSize: 16, fontWeight: "600" }}>Rename chat</Text>
            <TextInput
              autoFocus
              value={newTitle}
              onChangeText={setNewTitle}
              maxLength={120}
              selectTextOnFocus
              returnKeyType="done"
              onSubmitEditing={() => void rename()}
              accessibilityLabel="Chat name"
              style={{ color: C.ink, borderWidth: 1, borderColor: C.ember, borderRadius: 10, paddingHorizontal: 12, minHeight: 44, fontSize: 16 }}
            />
            <View style={[s.row, { justifyContent: "flex-end" }]}>
              <Button kind="ghost" title="Cancel" onPress={() => setRenaming(null)} />
              <Button title="Save" onPress={() => void rename()} />
            </View>
          </View>
        </View>
      </Modal>
    </ScrollView>
  );
}

interface Draft {
  content: string;
  thinking: string;
  files: FileAction[];
}

function Conversation({
  api,
  chat,
  persona,
  initial,
  onBack,
  onError,
}: {
  api: Outpost;
  chat: ChatT;
  persona: Persona | null;
  /** What the message box starts with (a share). */
  initial?: ChatStart | null;
  onBack: () => void;
  onError: (e: unknown) => void;
}) {
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [input, setInput] = useState(initial?.text ?? "");
  const [images, setImages] = useState<string[]>(initial?.images ?? []);
  const [error, setError] = useState<string | null>(null);
  const stop = useRef<(() => void) | null>(null);
  const [attach, setAttach] = useState(false);
  const [fromGallery, setFromGallery] = useState(false);
  const scroller = useRef<ScrollView | null>(null);
  const [mic, setMic] = useState<Listening | null>(null);
  const [handsFree, setHandsFree] = useState(false);
  const [saying, setSaying] = useState<string | null>(null);
  useEffect(() => onSaying(setSaying), []);
  // Files in chats: the chat's folders, its changes, and whether it asks first.
  const chatFiles = useChatFiles(api, chat.id);
  const [fileMode, setFileMode] = useState<"ask" | "auto">(chat.file_mode ?? "ask");
  const [filesOpen, setFilesOpen] = useState(false);
  const [toolsOpen, setToolsOpen] = useState(false);
  const chatTools = useChatToolNames(api, chat.id);
  async function changeFileMode(m: "ask" | "auto") {
    setFileMode(m);
    try {
      await api.json("PUT", `/api/chats/${chat.id}/file-mode`, { mode: m });
    } catch (e) {
      setFileMode(m === "ask" ? "auto" : "ask");
      setError(e instanceof Error ? e.message : "Couldn't change that.");
    }
  }
  // The model this chat needs, if it isn't downloaded (a persona copied from another PC).
  const [chatModels, setChatModels] = useState<string[] | null>(null);
  // The chat's own model (over the persona's) and the persona's, as changed here.
  const [chatModel, setChatModel] = useState<string | null>(chat.model ?? null);
  const [personaModel, setPersonaModel] = useState<string | null>(persona?.model ?? null);
  const [modelOpen, setModelOpen] = useState(false);
  const [docsOpen, setDocsOpen] = useState(false);
  // A command the reply wants to run, waiting for Allow or Skip.
  const [asking, setAsking] = useState<{ id: string; command: string; folder: string; always: string | null } | null>(null);
  async function answerAsk(id: string, answer: { ok: boolean; always?: boolean; cannot_ask?: boolean }) {
    setAsking(null);
    try {
      await api.json("POST", `/api/chats/${chat.id}/file-answers/${id}`, answer);
    } catch {
      // The reply stopped waiting (it skips it).
    }
  }
  useEffect(() => {
    api.get<{ installed: { model: string; badges: string[] }[] }>("/api/models").then(
      (o) => setChatModels(o.installed.filter((m) => !m.badges.includes("embedding")).map((m) => m.model)),
      () => undefined,
    );
  }, [api]);
  const wanted = chatModel ?? personaModel ?? null;
  const missingModel =
    wanted && chatModels && !chatModels.some((m) => m.toLowerCase() === wanted.toLowerCase()) ? wanted : null;
  async function pickModel(m: string) {
    try {
      if (!chatModel && persona && !persona.owner_name) {
        await api.json("PATCH", `/api/personas/${persona.id}`, { model: m });
        setPersonaModel(m);
      } else {
        await api.json("PATCH", `/api/chats/${chat.id}`, { model: m });
        setChatModel(m);
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't change the model.");
    }
  }
  // Leaving the chat stops anything being read aloud or listened to.
  useEffect(
    () => () => {
      stopSaying();
    },
    [],
  );

  function readAloud(key: string, text: string) {
    if (saying === key) return stopSaying();
    say(api, text, persona?.id ?? null, key).catch((e) => setError(e instanceof Error ? e.message : "The reply couldn't be read aloud."));
  }

  /** Talk instead of typing: the words land in the box, to check before sending. */
  function toggleMic() {
    if (mic) return mic.stop();
    stopSaying();
    setError(null);
    const before = input.trim();
    const join = (t: string) => (before ? `${before} ${t}` : t);
    const l = hear(api, { partial: (t) => setInput(join(t)) });
    setMic(l);
    l.done.then(
      (t) => {
        setMic(null);
        setInput(t ? join(t) : before);
      },
      (e) => {
        setMic(null);
        setInput(before);
        setError(e instanceof Error ? e.message : "The microphone stopped.");
      },
    );
  }

  // Long-press a message to delete it (yours take their reply, and what was learned).
  const removeMessage = (m: Message) =>
    Alert.alert(
      m.role === "user" ? "Delete this message?" : "Delete this reply?",
      m.role === "user" ? "Its reply goes too, and anything the persona learned from it is forgotten." : undefined,
      [
        { text: "Cancel", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () =>
            void api
              .json<{ removed: string[] }>("DELETE", `/api/chats/${chat.id}/messages/${m.id}`)
              .then((r) => setMessages((list) => list.filter((x) => !r.removed.includes(x.id))))
              .catch((e) => {
                onError(e);
                setError(e instanceof Error ? e.message : "Couldn't delete it.");
              }),
        },
      ],
    );

  const reload = useCallback(
    () =>
      api.get<{ messages: Message[] }>(`/api/chats/${chat.id}`).then(
        (r) => setMessages(r.messages),
        (e) => {
          onError(e);
          setError(e instanceof Error ? e.message : "Couldn't load the chat.");
        },
      ),
    [api, chat.id, onError],
  );
  useEffect(() => {
    void reload().then(() => follow());
    return () => stop.current?.();
  }, [reload]);

  // Keep the newest message in view when the keyboard opens.
  useEffect(() => {
    const sub = Keyboard.addListener("keyboardDidShow", () => scroller.current?.scrollToEnd({ animated: true }));
    return () => sub.remove();
  }, []);

  // Android's back button returns to the chat list rather than leaving the app.
  useEffect(() => {
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      onBack();
      return true;
    });
    return () => sub.remove();
  }, [onBack]);

  async function pick() {
    setAttach(false);
    const r = await ImagePicker.launchImageLibraryAsync({ mediaTypes: ["images"], base64: true, quality: 0.85, allowsMultipleSelection: true, selectionLimit: MAX_IMAGES });
    if (r.canceled) return;
    const picked = r.assets
      .filter((a) => a.base64)
      .map((a) => `data:${a.mimeType ?? "image/jpeg"};base64,${a.base64}`);
    setImages((cur) => [...cur, ...picked].slice(0, MAX_IMAGES));
  }

  /** What a streamed reply's events do here (sent from this phone, or followed). */
  function replyEvents(reply: { message: Message | null }) {
    return (name: string, data: unknown) => {
      const d = data as { text?: string; message?: unknown; user_message?: Message };
      if (name === "done" && d.message && typeof d.message === "object") reply.message = d.message as Message;
      if (name === "start" && d.user_message) setMessages((m) => (m.some((x) => x.id === d.user_message!.id) ? m : [...m, d.user_message!]));
      else if (name === "token") setDraft((x) => (x ? { ...x, content: x.content + (d.text ?? "") } : x));
      else if (name === "thinking") setDraft((x) => (x ? { ...x, thinking: x.thinking + (d.text ?? "") } : x));
      else if (name === "file") {
        const f = data as FileAction;
        setDraft((x) => (x ? { ...x, files: [...x.files, f] } : x));
        if (f.change_id) void chatFiles.reload();
      } else if (name === "confirm") {
        // A command the reply wants to run in a folder (asks here; other asks are skipped).
        const c = data as { id: string; kind?: string; command?: string; folder?: string; always?: string | null };
        if (c.kind === "command") setAsking({ id: c.id, command: c.command ?? "", folder: c.folder ?? "", always: c.always ?? null });
        else void answerAsk(c.id, { ok: false, cannot_ask: true });
      } else if (name === "error") setError(String((d as { message?: string }).message ?? "The reply failed."));
    };
  }

  /** A reply being written in this chat from another device (the PC): show it as it's written. */
  function follow() {
    if (stop.current) return;
    const reply: { message: Message | null } = { message: null };
    const events = replyEvents(reply);
    let started = false;
    const mine: () => void = api.stream(
      `/api/chats/${chat.id}/live`,
      null,
      {
        event: (name, data) => {
          if (!started) {
            started = true;
            setDraft((x) => x ?? { content: "", thinking: "", files: [] });
          }
          // Requests for the device that started it aren't for this phone.
          if (name !== "file_request") events(name, data);
        },
        done: () => {
          if (stop.current === mine) stop.current = null;
          if (!started) return; // nothing being written
          setDraft(null);
          setAsking(null);
          void reload();
          void chatFiles.reload();
        },
        error: () => {
          if (stop.current === mine) stop.current = null;
          if (started) {
            setDraft(null);
            setAsking(null);
            void reload();
          }
        },
      },
      "GET",
    );
    stop.current = mine;
  }

  function send() {
    const text = input.trim();
    if ((!text && images.length === 0) || draft) return;
    const sending = images;
    setInput("");
    setImages([]);
    setError(null);
    setDraft({ content: "", thinking: "", files: [] });
    stopSaying();
    // Still checking for a reply written elsewhere: this one replaces it.
    stop.current?.();
    const reply: { message: Message | null } = { message: null };
    stop.current = api.stream(`/api/chats/${chat.id}/messages`, { content: text, images: sending, ...clock(), use_pc_tools: true }, {
      event: replyEvents(reply),
      done: () => {
        setDraft(null);
        setAsking(null);
        stop.current = null;
        void reload();
        void chatFiles.reload();
        const r = reply.message;
        if (persona?.speak_replies && r?.content) readAloud(r.id, r.content);
      },
      error: (e) => {
        setDraft(null);
        stop.current = null;
        onError(e);
        setError(e.message);
        setInput(text);
        setImages(sending);
      },
    });
  }

  return (
    // The app pads itself above the keyboard (App.tsx), so the chat above shrinks to fit.
    <View style={s.screen}>
      <View style={[s.row, { padding: 12, borderBottomWidth: 1, borderBottomColor: C.line }]}>
        <Pressable accessibilityRole="button" accessibilityLabel="Back to chats" onPress={onBack} style={[s.chip, { minHeight: 40 }]}>
          <Text style={{ color: C.ink }}>‹ Chats</Text>
        </Pressable>
        {persona && <Avatar name={persona.name} color={persona.avatar_color} size={28} />}
        <Text numberOfLines={1} style={{ color: C.ink, fontSize: 16, flex: 1 }}>
          {chat.title}
        </Text>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Talk hands-free"
          disabled={!!draft}
          onPress={() => {
            mic?.cancel();
            stopSaying();
            setHandsFree(true);
          }}
          style={[s.chip, { minHeight: 40, opacity: draft ? 0.45 : 1 }]}
        >
          <Text style={{ color: C.ink }}>Talk</Text>
        </Pressable>
      </View>
      <ScrollView
        ref={scroller}
        style={{ flex: 1 }}
        contentContainerStyle={[s.pad, { paddingBottom: 24 }]}
        keyboardShouldPersistTaps="handled"
        onContentSizeChange={() => scroller.current?.scrollToEnd({ animated: false })}
        onLayout={() => scroller.current?.scrollToEnd({ animated: false })}
      >
        {messages.length === 0 && !draft && <P muted>{persona ? `Say hello to ${persona.name}.` : "Start the chat."}</P>}
        {messages
          .filter((m) => m.role !== "system")
          .map((m) => (
            <View key={m.id} style={{ gap: 8 }}>
            <Bubble
              api={api}
              role={m.role}
              content={m.content}
              thinking={m.thinking}
              attachments={m.attachments.map((a) => a.id)}
              sources={m.sources}
              onDelete={draft ? undefined : () => removeMessage(m)}
              speaking={saying === m.id}
              onSpeak={m.role === "assistant" ? () => readAloud(m.id, m.content) : undefined}
            />
            {m.files && m.files.length > 0 && <FileActions api={api} chatId={chat.id} actions={m.files} files={chatFiles} />}
            </View>
          ))}
        {draft && <Bubble api={api} role="assistant" content={draft.content || "…"} thinking={draft.thinking || null} attachments={[]} />}
        {draft && draft.files.length > 0 && <FileActions api={api} chatId={chat.id} actions={draft.files} files={chatFiles} live />}
      </ScrollView>
      <View style={{ padding: 12, gap: 8, borderTopWidth: 1, borderTopColor: C.line }}>
        {missingModel && (
          <View accessibilityRole="alert" style={[s.card, { gap: 8, borderColor: C.amber }]}>
            <Text style={{ color: C.ink }}>
              {!chatModel && persona ? `${persona.name} uses ` : "This chat uses "}
              {missingModel}, which isn't downloaded on your Outpost.{" "}
              {chatModels?.length ? "Pick another:" : "Get a chat model in Models on the PC first."}
            </Text>
            <View style={[s.row, { flexWrap: "wrap" }]}>
              {(chatModels ?? []).slice(0, 6).map((m) => (
                <Chip key={m} label={m} onPress={() => void pickModel(m)} />
              ))}
            </View>
          </View>
        )}
        {asking && (
          <View accessibilityRole="alert" style={[s.card, { gap: 8, borderColor: C.amber }]}>
            <Text style={{ color: C.ink }}>Run this in {asking.folder}?</Text>
            <Text style={{ color: C.soft, fontFamily: "monospace", backgroundColor: C.raised, padding: 8, borderRadius: 8 }}>{asking.command}</Text>
            <View style={[s.row, { flexWrap: "wrap" }]}>
              <Chip label="Allow" active onPress={() => void answerAsk(asking.id, { ok: true })} />
              {asking.always && <Chip label={`Always allow ${asking.always} here`} onPress={() => void answerAsk(asking.id, { ok: true, always: true })} />}
              <Chip label="Skip" onPress={() => void answerAsk(asking.id, { ok: false })} />
            </View>
          </View>
        )}
        <ErrorText>{error}</ErrorText>
        {error && !draft && (
          <View style={s.row}>
            <Chip label={`Try another model${wanted ? ` (now ${wanted})` : ""}`} onPress={() => setModelOpen(true)} />
          </View>
        )}
        {attach && (
          <View style={[s.row, { flexWrap: "wrap" }]}>
            <Chip
              label="From gallery"
              onPress={() => {
                setAttach(false);
                setFromGallery(true);
              }}
            />
            <Chip label="Phone photos" onPress={() => void pick()} />
            <Chip
              label={wanted ? `Model: ${wanted.length > 22 ? `${wanted.slice(0, 21)}…` : wanted}` : "Model"}
              onPress={() => {
                setAttach(false);
                setModelOpen(true);
              }}
            />
            <Chip
              label="Documents"
              onPress={() => {
                setAttach(false);
                setDocsOpen(true);
              }}
            />
            <Chip
              label={chatTools.names.length ? `Tools (${chatTools.names.length})` : "Tools"}
              onPress={() => {
                setAttach(false);
                setToolsOpen(true);
              }}
            />
            <Chip
              label={chatFiles.folders.length ? `Files and folders (${chatFiles.folders.length})` : "Files and folders"}
              onPress={() => {
                setAttach(false);
                setFilesOpen(true);
              }}
            />
          </View>
        )}
        {images.length > 0 && (
          <View style={s.row}>
            {images.map((uri, i) => (
              <Pressable key={i} accessibilityLabel="Remove picture" onPress={() => setImages((cur) => cur.filter((_, j) => j !== i))}>
                <Image source={{ uri }} style={{ width: 52, height: 52, borderRadius: 8 }} />
              </Pressable>
            ))}
          </View>
        )}
        <View style={{ flexDirection: "row", alignItems: "flex-end", gap: 8 }}>
          {/* Attach and talk, stacked, so the message box gets the width. */}
          <View style={{ gap: 6 }}>
            <RoundButton label="Add pictures or files" onPress={() => setAttach((a) => !a)} active={attach}>
              <PlusIcon size={20} color={attach ? C.onEmber : C.soft} />
            </RoundButton>
            <RoundButton label={mic ? "Stop listening" : "Talk instead of typing"} onPress={toggleMic} active={!!mic} disabled={!!draft}>
              {mic ? <StopIcon size={18} color={C.onEmber} /> : <MicIcon size={20} color={C.soft} />}
            </RoundButton>
          </View>
          <TextInput
            value={input}
            onChangeText={setInput}
            placeholder={mic ? "Listening…" : persona ? `Message ${persona.name}, or /draw…` : "Message, or /draw…"}
            placeholderTextColor={C.muted}
            multiline
            onFocus={() => setTimeout(() => scroller.current?.scrollToEnd({ animated: true }), 250)}
            style={{
              flex: 1,
              minHeight: 82,
              maxHeight: 160,
              borderRadius: 20,
              borderWidth: 1,
              borderColor: mic ? C.ember : C.line,
              backgroundColor: C.deep,
              color: C.ink,
              fontSize: 16,
              paddingHorizontal: 14,
              paddingTop: 12,
              paddingBottom: 12,
              textAlignVertical: "top",
            }}
          />
          {draft ? (
            <RoundButton
              label="Stop the reply"
              onPress={() => {
                // The reply ends at its next step (a running build is stopped too).
                void api.json("POST", `/api/chats/${chat.id}/stop`).catch(() => undefined);
                stop.current?.();
              }}
              size={48}
            >
              <StopIcon size={20} color={C.ink} />
            </RoundButton>
          ) : (
            <RoundButton label="Send" onPress={send} size={48} filled disabled={!input.trim() && images.length === 0}>
              <SendIcon size={22} color={C.onEmber} />
            </RoundButton>
          )}
        </View>
      </View>
      {docsOpen && <ChatDocsSheet api={api} chatId={chat.id} onClose={() => setDocsOpen(false)} />}
      {modelOpen && (
        <ModelSheet
          api={api}
          chatId={chat.id}
          chatModel={chatModel}
          persona={persona}
          personaModel={personaModel}
          onChanged={(c) => {
            setChatModel(c.chat);
            setPersonaModel(c.persona);
            setError(null);
          }}
          onClose={() => setModelOpen(false)}
        />
      )}
      {toolsOpen && (
        <ToolsSheet
          api={api}
          chatId={chat.id}
          onClose={() => {
            setToolsOpen(false);
            void chatTools.reload();
          }}
        />
      )}
      {filesOpen && (
        <FilesSheet api={api} chatId={chat.id} mode={fileMode} files={chatFiles} onMode={(m) => void changeFileMode(m)} onClose={() => setFilesOpen(false)} />
      )}
      {handsFree && (
        <HandsFree
          api={api}
          chatId={chat.id}
          persona={persona}
          onTurn={() => void reload()}
          onClose={() => {
            setHandsFree(false);
            void reload();
          }}
        />
      )}
      {fromGallery && (
        <GalleryPicker
          api={api}
          onClose={() => setFromGallery(false)}
          onPick={(m) => {
            setFromGallery(false);
            api.image(`/api/media/${m.id}`).then(
              (uri) => setImages((cur) => [...cur, uri].slice(0, MAX_IMAGES)),
              (e) => setError(e instanceof Error ? e.message : "That picture couldn't be attached."),
            );
          }}
        />
      )}
    </View>
  );
}

function Bubble({
  api,
  role,
  content,
  thinking,
  attachments,
  sources,
  onDelete,
  speaking,
  onSpeak,
}: {
  api: Outpost;
  role: Message["role"];
  content: string;
  thinking: string | null;
  attachments: string[];
  sources?: Message["sources"];
  onDelete?: () => void;
  speaking?: boolean;
  onSpeak?: () => void;
}) {
  const [showThinking, setShowThinking] = useState(false);
  const user = role === "user";
  const jobIds = user ? [] : [...content.matchAll(new RegExp(JOB_CARD.source, "g"))].map((m) => m[1]);
  const text = jobIds.length ? content.replace(new RegExp(JOB_CARD.source, "g"), "").trim() : content;
  return (
    <Pressable
      onLongPress={onDelete}
      delayLongPress={450}
      accessibilityHint={onDelete ? "Long-press to delete" : undefined}
      style={{ alignSelf: user ? "flex-end" : "stretch", maxWidth: user ? "85%" : "100%", gap: 6 }}
    >
      {attachments.length > 0 && (
        <View style={[s.row, { justifyContent: user ? "flex-end" : "flex-start", flexWrap: "wrap" }]}>
          {attachments.map((id) => (
            <AuthedImage key={id} api={api} path={`/api/files/${id}`} style={{ width: 120, height: 120, borderRadius: 10 }} />
          ))}
        </View>
      )}
      {thinking ? (
        <Pressable onPress={() => setShowThinking((v) => !v)}>
          <Text style={{ color: C.muted, fontSize: 13 }}>{showThinking ? thinking : "Show reasoning"}</Text>
        </Pressable>
      ) : null}
      {text ? (
        <Text
          style={{
            color: C.ink,
            fontSize: 16,
            lineHeight: 23,
            backgroundColor: user ? C.raised : "transparent",
            borderRadius: 14,
            padding: user ? 12 : 0,
          }}
        >
          {text}
        </Text>
      ) : null}
      {jobIds.map((id) => (
        <JobCard key={id} api={api} jobId={id} />
      ))}
      {sources && sources.length > 0 ? (
        <Text style={{ color: C.muted, fontSize: 13 }}>
          Sources: {sources.map((x) => `[${x.n}] ${x.name}${x.page != null ? `, page ${x.page}` : ""}`).join(" · ")}
        </Text>
      ) : null}
      {onSpeak && text ? (
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={speaking ? "Stop reading aloud" : "Read aloud"}
          onPress={onSpeak}
          hitSlop={8}
          style={{ alignSelf: "flex-start", paddingVertical: 4 }}
        >
          <Text style={{ color: speaking ? C.ember : C.muted, fontSize: 13 }}>{speaking ? "■ Stop" : "▶ Listen"}</Text>
        </Pressable>
      ) : null}
    </Pressable>
  );
}

/** A reply that started a picture carries `[[outpost-job:<id>]]`. */
const JOB_CARD = /\[\[outpost-job:([0-9a-f-]{36})\]\]/;

type JobView = {
  status: "queued" | "running" | "done" | "failed" | "cancelled";
  error: string | null;
  position: number | null;
  progress: { phase: string; step: number; steps: number; percent: number | null } | null;
  media: { id: string; kind: "image" | "video" }[];
};

/** The picture a /draw started: waiting, drawing, then the picture itself. */
function JobCard({ api, jobId }: { api: Outpost; jobId: string }) {
  const [job, setJob] = useState<JobView | null>(null);
  const [gone, setGone] = useState(false);
  const finished = job != null && job.status !== "queued" && job.status !== "running";
  useEffect(() => {
    let live = true;
    const load = () =>
      api.get<JobView>(`/api/jobs/${jobId}`).then(
        (j) => live && setJob(j),
        () => live && setGone(true),
      );
    void load();
    if (finished) return;
    const t = setInterval(() => void load(), 2000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, jobId, finished]);
  const card = { borderWidth: 1, borderColor: C.line, borderRadius: 14, padding: 12, gap: 8, backgroundColor: C.surface };
  if (gone) return <Text style={{ color: C.muted }}>This picture is no longer here.</Text>;
  if (!job) return <View style={[card, { height: 60 }]} />;
  if (job.status === "done") {
    return (
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {job.media.map((m) =>
          m.kind === "image" ? (
            <AuthedImage key={m.id} api={api} path={`/api/media/${m.id}`} style={{ width: 280, height: 280, borderRadius: 14 }} />
          ) : (
            <Text key={m.id} style={{ color: C.soft }}>
              Video ready: open it in the Gallery.
            </Text>
          ),
        )}
      </View>
    );
  }
  if (job.status === "failed") return <Text style={{ color: C.danger }}>Couldn't draw it: {job.error ?? "unknown error"}</Text>;
  if (job.status === "cancelled") return <Text style={{ color: C.muted }}>Stopped.</Text>;
  const p = job.progress;
  const f = p ? (p.percent != null ? p.percent / 100 : p.steps > 0 ? p.step / p.steps : 0) : 0;
  const label =
    job.status === "queued"
      ? job.position
        ? `Waiting: ${job.position} ahead`
        : "Starting…"
      : `${p?.phase ?? "Drawing"}${p && p.steps > 0 ? ` · step ${p.step} of ${p.steps}` : ""}`;
  return (
    <View style={card} accessibilityLiveRegion="polite">
      <Text style={{ color: C.muted }}>{label}</Text>
      <View style={{ height: 6, borderRadius: 3, backgroundColor: C.raised, overflow: "hidden", flexDirection: "row" }}>
        <View style={{ flex: Math.min(1, f), backgroundColor: C.ember }} />
        <View style={{ flex: 1 - Math.min(1, f) }} />
      </View>
    </View>
  );
}

/** A round icon button for the message bar. */
function RoundButton({
  label,
  onPress,
  children,
  size = 38,
  filled,
  active,
  disabled,
}: {
  label: string;
  onPress: () => void;
  children: ReactNode;
  size?: number;
  /** The main action (send): always ember. */
  filled?: boolean;
  /** On (listening, attach menu open). */
  active?: boolean;
  disabled?: boolean;
}) {
  const lit = filled || active;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled, selected: !!active }}
      onPress={onPress}
      disabled={disabled}
      hitSlop={4}
      style={({ pressed }) => ({
        width: size,
        height: size,
        borderRadius: size / 2,
        alignItems: "center",
        justifyContent: "center",
        backgroundColor: lit ? C.ember : C.raised,
        opacity: disabled ? 0.4 : pressed ? 0.75 : 1,
      })}
    >
      {children}
    </Pressable>
  );
}

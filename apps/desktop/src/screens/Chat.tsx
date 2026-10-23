import { useCallback, useEffect, useRef, useState, type ClipboardEvent, type DragEvent, type KeyboardEvent } from "react";
import {
  api,
  ApiError,
  MAX_IMAGES,
  streamMessage,
  type Chat as ChatT,
  type InstalledModel,
  type Message,
  type Persona,
} from "../api";
import { AttachmentImage, prepareImage, type PendingImage } from "../components/images";
import { Markdown } from "../components/markdown";
import { Avatar, Button, ErrorNote } from "../components/ui";

interface Draft {
  content: string;
  thinking: string;
  model: string;
}

export default function Chat({ token, onUnauthorized }: { token: string; onUnauthorized: () => void }) {
  const [chats, setChats] = useState<ChatT[]>([]);
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [activeId, setActiveId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [draft, setDraft] = useState<Draft | null>(null);
  // The chat a reply is streaming into. You can switch chats meanwhile; the reply
  // only shows in its own chat (it is saved on the server either way).
  const [streamingId, setStreamingId] = useState<string | null>(null);
  const activeRef = useRef<string | null>(null);
  const [input, setInput] = useState("");
  const [images, setImages] = useState<PendingImage[]>([]);
  const [dragging, setDragging] = useState(false);
  const [installed, setInstalled] = useState<InstalledModel[]>([]);
  const [error, setError] = useState<string | null>(null);
  const filePicker = useRef<HTMLInputElement | null>(null);
  const abort = useRef<AbortController | null>(null);
  const bottom = useRef<HTMLDivElement | null>(null);

  const fail = useCallback(
    (err: unknown) => {
      if (err instanceof ApiError && err.status === 401) return onUnauthorized();
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    },
    [onUnauthorized],
  );

  const loadLists = useCallback(async () => {
    try {
      const [c, p] = await Promise.all([api.chats(token), api.personas(token)]);
      setChats(c);
      setPersonas(p);
      return c;
    } catch (err) {
      fail(err);
      return [];
    }
  }, [token, fail]);

  useEffect(() => {
    void loadLists().then((c) => {
      if (c.length > 0) setActiveId((id) => id ?? c[0].id);
    });
  }, [loadLists]);

  useEffect(() => {
    activeRef.current = activeId;
    if (!activeId) {
      setMessages([]);
      return;
    }
    api.chat(token, activeId).then((r) => setMessages(r.messages), fail);
  }, [activeId, token, fail]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [messages, draft]);

  // Which models can see, to warn before sending an image to one that can't.
  useEffect(() => {
    api.modelsOverview(token).then(
      (o) => setInstalled(o.installed),
      () => undefined,
    );
  }, [token]);

  const active = chats.find((c) => c.id === activeId) ?? null;
  const persona = personas.find((p) => p.id === active?.persona_id) ?? null;
  const modelName =
    active?.model ?? persona?.model ?? installed.find((m) => !m.badges.includes("embedding"))?.model ?? null;
  const modelInfo = installed.find((m) => m.model === modelName);
  const blind = images.length > 0 && modelInfo !== undefined && !modelInfo.badges.includes("vision");

  async function addFiles(files: File[]) {
    const room = MAX_IMAGES - images.length;
    if (files.length > room) setError(`Attach up to ${MAX_IMAGES} images at a time.`);
    const added: PendingImage[] = [];
    for (const f of files.slice(0, Math.max(0, room))) {
      try {
        added.push(await prepareImage(f));
      } catch (err) {
        setError(err instanceof Error ? err.message : "That image couldn't be added.");
      }
    }
    if (added.length) setImages((cur) => [...cur, ...added].slice(0, MAX_IMAGES));
  }

  function onPaste(e: ClipboardEvent<HTMLTextAreaElement>) {
    const files = Array.from(e.clipboardData.files).filter((f) => f.type.startsWith("image/"));
    if (files.length) {
      e.preventDefault();
      void addFiles(files);
    }
  }

  function onDrop(e: DragEvent<HTMLElement>) {
    e.preventDefault();
    setDragging(false);
    const files = Array.from(e.dataTransfer.files).filter((f) => f.type.startsWith("image/"));
    if (files.length) void addFiles(files);
  }

  async function newChat(personaId?: string) {
    try {
      const chat = await api.createChat(token, personaId ?? personas[0]?.id);
      setChats((c) => [chat, ...c]);
      setActiveId(chat.id);
    } catch (err) {
      fail(err);
    }
  }

  async function switchPersona(personaId: string) {
    if (!active) return;
    try {
      const chat = await api.updateChat(token, active.id, { persona_id: personaId });
      setChats((cs) => cs.map((c) => (c.id === chat.id ? chat : c)));
    } catch (err) {
      fail(err);
    }
  }

  async function removeChat(id: string) {
    if (!window.confirm("Delete this chat? Its images, and anything Outpost learned from it, go too.")) return;
    try {
      await api.deleteChat(token, id);
      setChats((cs) => cs.filter((c) => c.id !== id));
      if (activeId === id) setActiveId(null);
    } catch (err) {
      fail(err);
    }
  }

  async function send() {
    const text = input.trim();
    if ((!text && images.length === 0) || draft) return;
    const sending = images;
    setError(null);
    let chatId = activeId;
    if (!chatId) {
      try {
        const chat = await api.createChat(token, personas[0]?.id);
        setChats((c) => [chat, ...c]);
        setActiveId(chat.id);
        activeRef.current = chat.id;
        chatId = chat.id;
      } catch (err) {
        return fail(err);
      }
    }
    setInput("");
    setImages([]);
    setDraft({ content: "", thinking: "", model: "" });
    setStreamingId(chatId);
    const here = () => activeRef.current === chatId;
    const controller = new AbortController();
    abort.current = controller;
    try {
      await streamMessage(
        token,
        chatId,
        text,
        sending.map((i) => i.dataUrl),
        {
          start: ({ user_message, model }) => {
            if (here()) setMessages((m) => (m.some((x) => x.id === user_message.id) ? m : [...m, user_message]));
            setDraft((d) => (d ? { ...d, model } : d));
          },
          thinking: (t) => setDraft((d) => (d ? { ...d, thinking: d.thinking + t } : d)),
          token: (t) => setDraft((d) => (d ? { ...d, content: d.content + t } : d)),
          done: ({ message, error }) => {
            if (here()) setMessages((m) => (m.some((x) => x.id === message.id) ? m : [...m, message]));
            if (error) setError(error);
          },
          error: (message) => setError(message),
        },
        controller.signal,
      );
    } catch (err) {
      if (!(err instanceof DOMException && err.name === "AbortError")) {
        setInput(text);
        setImages(sending);
        fail(err);
      }
    } finally {
      setDraft(null);
      setStreamingId(null);
      abort.current = null;
      void loadLists(); // titles and order change after a message
    }
  }

  function stop() {
    // The reply keeps generating and is saved on the server; we just stop following it.
    abort.current?.abort();
    if (activeId) window.setTimeout(() => api.chat(token, activeId).then((r) => setMessages(r.messages), fail), 1500);
  }

  function onKey(e: KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  }

  return (
    <div className="flex h-full min-h-0">
      <aside aria-label="Chats" className="flex w-72 shrink-0 flex-col gap-3 border-r border-line bg-surface p-4">
        <Button onClick={() => void newChat()}>+ New chat</Button>
        <ul className="flex min-h-0 flex-1 flex-col gap-1 overflow-auto">
          {chats.map((c) => {
            const p = personas.find((x) => x.id === c.persona_id);
            return (
              <li key={c.id} className="group flex items-center gap-1">
                <button
                  type="button"
                  onClick={() => setActiveId(c.id)}
                  className={`flex min-w-0 flex-1 flex-col rounded-lg px-3 py-2 text-left ${
                    c.id === activeId ? "bg-raised" : "hover:bg-raised/60"
                  }`}
                >
                  <span className="truncate text-sm">{c.title}</span>
                  <span className="truncate text-xs text-muted">{p?.name ?? "No persona"}</span>
                </button>
                <button
                  type="button"
                  aria-label={`Delete ${c.title}`}
                  onClick={() => void removeChat(c.id)}
                  className="rounded-md px-2 py-1 text-muted opacity-0 group-hover:opacity-100 hover:text-danger focus:opacity-100"
                >
                  ×
                </button>
              </li>
            );
          })}
        </ul>
      </aside>

      <section
        className={`relative flex min-w-0 flex-1 flex-col ${dragging ? "ring-2 ring-ember ring-inset" : ""}`}
        onDragOver={(e) => {
          if (Array.from(e.dataTransfer.types).includes("Files")) {
            e.preventDefault();
            setDragging(true);
          }
        }}
        onDragLeave={(e) => {
          if (e.currentTarget === e.target) setDragging(false);
        }}
        onDrop={onDrop}
      >
        <header className="flex flex-wrap items-center gap-3 border-b border-line px-6 py-3">
          {persona && <Avatar name={persona.name} color={persona.avatar_color} size={28} />}
          <label className="flex items-center gap-2 text-sm text-muted">
            Persona
            <select
              value={active?.persona_id ?? ""}
              disabled={!active || !!draft}
              onChange={(e) => void switchPersona(e.target.value)}
              className="min-h-9 rounded-lg border border-line bg-surface px-2 text-ink"
            >
              {personas.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>
          <span className="font-mono text-xs text-muted">
            {draft?.model || persona?.model || "default model"}
            {persona?.think ? " · thinking on" : ""}
          </span>
        </header>

        <div className="min-h-0 flex-1 overflow-auto">
          <div className="mx-auto flex max-w-3xl flex-col gap-6 px-6 py-8">
            {messages.length === 0 && !draft && (
              <p className="mt-16 text-center text-muted">
                {persona ? `Say hello to ${persona.name}.` : "Start a chat."} Everything stays on this PC.
              </p>
            )}
            {messages.map((m) => (
              <Bubble key={m.id} token={token} message={m} persona={persona} />
            ))}
            {draft && streamingId === activeId && (
              <Bubble
                token={token}
                message={{ role: "assistant", content: draft.content, thinking: draft.thinking, attachments: [] }}
                persona={persona}
                live
              />
            )}
            <div ref={bottom} />
          </div>
        </div>

        <div className="mx-auto w-full max-w-3xl px-6 pb-6">
          <ErrorNote>{error}</ErrorNote>
          <div className="mt-2 flex flex-col gap-2 rounded-2xl border border-line bg-surface p-3">
            {images.length > 0 && (
              <ul className="flex flex-wrap gap-2" aria-label="Images to send">
                {images.map((img) => (
                  <li key={img.key} className="relative">
                    <img src={img.dataUrl} alt={img.name} className="h-16 w-16 rounded-lg border border-line object-cover" />
                    <button
                      type="button"
                      aria-label={`Remove ${img.name}`}
                      onClick={() => setImages((cur) => cur.filter((i) => i.key !== img.key))}
                      className="absolute -top-2 -right-2 flex size-6 items-center justify-center rounded-full border border-line bg-raised text-sm text-muted hover:text-ink"
                    >
                      ×
                    </button>
                  </li>
                ))}
              </ul>
            )}
            {blind && (
              <p className="text-sm text-amber" role="status">
                {modelName} can't see images. Pick a model with vision for this persona, or remove the images.
              </p>
            )}
            <label className="sr-only" htmlFor="composer">
              Message
            </label>
            <textarea
              id="composer"
              rows={2}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKey}
              onPaste={onPaste}
              placeholder={persona ? `Message ${persona.name}… (Enter to send, Shift+Enter for a new line)` : "Message…"}
              className="w-full resize-none bg-transparent text-ink placeholder:text-muted/60 focus:outline-none"
            />
            <div className="flex items-center justify-end gap-2">
              <input
                ref={filePicker}
                type="file"
                accept="image/png,image/jpeg,image/webp,image/gif"
                multiple
                hidden
                onChange={(e) => {
                  void addFiles(Array.from(e.target.files ?? []));
                  e.target.value = "";
                }}
              />
              <Button
                variant="ghost"
                className="mr-auto"
                disabled={!!draft || images.length >= MAX_IMAGES}
                onClick={() => filePicker.current?.click()}
                title="Attach images (or paste / drop them)"
              >
                Attach image
              </Button>
              {draft ? (
                <Button variant="ghost" onClick={stop}>
                  Stop following
                </Button>
              ) : (
                <Button onClick={() => void send()} disabled={(!input.trim() && images.length === 0) || blind}>
                  Send
                </Button>
              )}
            </div>
          </div>
        </div>
      </section>
    </div>
  );
}

function Bubble({
  token,
  message,
  persona,
  live = false,
}: {
  token: string;
  message: Pick<Message, "role" | "content" | "thinking" | "attachments">;
  persona: Persona | null;
  live?: boolean;
}) {
  const { role, content, thinking, attachments } = message;
  if (role === "user") {
    return (
      <div className="flex max-w-[80%] flex-col items-end gap-2 self-end">
        {attachments.length > 0 && (
          <div className="flex flex-wrap justify-end gap-2">
            {attachments.map((a) => (
              <AttachmentImage key={a.id} token={token} id={a.id} alt="Attached image" />
            ))}
          </div>
        )}
        {content && <div className="rounded-2xl rounded-br-md bg-raised px-4 py-3 whitespace-pre-wrap">{content}</div>}
      </div>
    );
  }
  return (
    <div className="flex gap-3">
      <Avatar name={persona?.name ?? "AI"} color={persona?.avatar_color ?? "#FFB547"} size={32} />
      <div className="flex min-w-0 flex-1 flex-col gap-2">
        {thinking && (
          <details className="text-sm" open={live && !content}>
            <summary className="cursor-pointer text-muted">{live && !content ? "Thinking…" : "Show reasoning"}</summary>
            <p className="mt-2 border-l-2 border-line pl-3 whitespace-pre-wrap text-muted">{thinking}</p>
          </details>
        )}
        <div aria-live={live ? "polite" : undefined}>
          <Markdown text={content} />
          {live && <span className="ml-0.5 inline-block h-4 w-2 animate-pulse bg-ember align-middle" />}
        </div>
      </div>
    </div>
  );
}

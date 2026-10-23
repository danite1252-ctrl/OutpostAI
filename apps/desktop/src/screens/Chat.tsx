import { useCallback, useEffect, useRef, useState, type KeyboardEvent } from "react";
import { api, ApiError, streamMessage, type Chat as ChatT, type Message, type Persona } from "../api";
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
  const [input, setInput] = useState("");
  const [error, setError] = useState<string | null>(null);
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
    if (!activeId) {
      setMessages([]);
      return;
    }
    api.chat(token, activeId).then((r) => setMessages(r.messages), fail);
  }, [activeId, token, fail]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ block: "end" });
  }, [messages, draft]);

  const active = chats.find((c) => c.id === activeId) ?? null;
  const persona = personas.find((p) => p.id === active?.persona_id) ?? null;

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
    if (!window.confirm("Delete this chat?")) return;
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
    if (!text || draft) return;
    setError(null);
    let chatId = activeId;
    if (!chatId) {
      try {
        const chat = await api.createChat(token, personas[0]?.id);
        setChats((c) => [chat, ...c]);
        setActiveId(chat.id);
        chatId = chat.id;
      } catch (err) {
        return fail(err);
      }
    }
    setInput("");
    setDraft({ content: "", thinking: "", model: "" });
    const controller = new AbortController();
    abort.current = controller;
    try {
      await streamMessage(
        token,
        chatId,
        text,
        {
          start: ({ user_message, model }) => {
            setMessages((m) => [...m, user_message]);
            setDraft((d) => (d ? { ...d, model } : d));
          },
          thinking: (t) => setDraft((d) => (d ? { ...d, thinking: d.thinking + t } : d)),
          token: (t) => setDraft((d) => (d ? { ...d, content: d.content + t } : d)),
          done: ({ message, error }) => {
            setMessages((m) => [...m, message]);
            if (error) setError(error);
          },
          error: (message) => setError(message),
        },
        controller.signal,
      );
    } catch (err) {
      if (!(err instanceof DOMException && err.name === "AbortError")) {
        setInput(text);
        fail(err);
      }
    } finally {
      setDraft(null);
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

      <section className="flex min-w-0 flex-1 flex-col">
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
              <Bubble key={m.id} role={m.role} content={m.content} thinking={m.thinking} persona={persona} />
            ))}
            {draft && (
              <Bubble role="assistant" content={draft.content} thinking={draft.thinking} persona={persona} live />
            )}
            <div ref={bottom} />
          </div>
        </div>

        <div className="mx-auto w-full max-w-3xl px-6 pb-6">
          <ErrorNote>{error}</ErrorNote>
          <div className="mt-2 flex flex-col gap-2 rounded-2xl border border-line bg-surface p-3">
            <label className="sr-only" htmlFor="composer">
              Message
            </label>
            <textarea
              id="composer"
              rows={2}
              value={input}
              onChange={(e) => setInput(e.target.value)}
              onKeyDown={onKey}
              placeholder={persona ? `Message ${persona.name}… (Enter to send, Shift+Enter for a new line)` : "Message…"}
              className="w-full resize-none bg-transparent text-ink placeholder:text-muted/60 focus:outline-none"
            />
            <div className="flex justify-end gap-2">
              {draft ? (
                <Button variant="ghost" onClick={stop}>
                  Stop following
                </Button>
              ) : (
                <Button onClick={() => void send()} disabled={!input.trim()}>
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
  role,
  content,
  thinking,
  persona,
  live = false,
}: {
  role: Message["role"];
  content: string;
  thinking: string | null;
  persona: Persona | null;
  live?: boolean;
}) {
  if (role === "user") {
    return (
      <div className="max-w-[80%] self-end rounded-2xl rounded-br-md bg-raised px-4 py-3 whitespace-pre-wrap">{content}</div>
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
        <div className="leading-relaxed whitespace-pre-wrap" aria-live={live ? "polite" : undefined}>
          {content}
          {live && <span className="ml-0.5 inline-block h-4 w-2 animate-pulse bg-ember align-middle" />}
        </div>
      </div>
    </div>
  );
}

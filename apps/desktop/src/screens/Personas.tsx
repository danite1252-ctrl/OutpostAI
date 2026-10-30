import { useCallback, useEffect, useState, type FormEvent } from "react";
import {
  api,
  ApiError,
  docsApi,
  voiceApi,
  type Doc,
  type DirectoryEntry,
  type InstalledModel,
  type Memory,
  type MemorySettings,
  type Persona,
  type VoiceStatus,
} from "../api";
import { DocumentPicker } from "../components/documents";
import { Avatar, Button, ErrorNote, Field } from "../components/ui";

export default function Personas({ token }: { token: string }) {
  const [personas, setPersonas] = useState<Persona[]>([]);
  const [models, setModels] = useState<InstalledModel[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const [p, m] = await Promise.all([api.personas(token), api.modelsOverview(token)]);
      setPersonas(p);
      setModels(m.installed.filter((x) => !x.badges.includes("embedding")));
      setSelectedId((id) => id ?? p[0]?.id ?? null);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Can't load personas.");
    }
  }, [token]);

  useEffect(() => {
    void load();
  }, [load]);

  async function add() {
    try {
      const p = await api.createPersona(token, { name: "New persona", system_prompt: "" });
      setPersonas((ps) => [...ps, p]);
      setSelectedId(p.id);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  const selected = personas.find((p) => p.id === selectedId) ?? null;

  return (
    <div className="mx-auto flex max-w-6xl flex-col gap-8">
      <div className="flex flex-wrap gap-6">
        <section aria-label="Personas" className="flex w-full max-w-xs flex-col gap-3">
          <div className="flex items-center">
            <h1 className="flex-1 text-2xl font-semibold">Personas</h1>
            <Button onClick={() => void add()}>New</Button>
          </div>
          <ErrorNote>{error}</ErrorNote>
          {personas.map((p) => (
            <button
              key={p.id}
              type="button"
              onClick={() => setSelectedId(p.id)}
              className={`flex items-start gap-3 rounded-2xl border p-3 text-left ${
                p.id === selectedId ? "border-ember bg-raised" : "border-line bg-surface hover:bg-raised"
              }`}
            >
              <Avatar name={p.name} color={p.avatar_color} size={36} />
              <span className="flex min-w-0 flex-col">
                <span className="font-medium">{p.name}</span>
                {p.owner_name ? (
                  <span className="truncate text-xs text-pick">Shared by {p.owner_name}</span>
                ) : (
                  <span className="truncate font-mono text-xs text-muted">
                    persona:{p.slug}
                    {p.shared ? <span className="ml-1 font-sans text-pick">· shared</span> : null}
                  </span>
                )}
                <span className="truncate text-xs text-muted">{p.model ?? "default model"}</span>
              </span>
            </button>
          ))}
        </section>

        {selected?.owner_name && (
          <SharedPersona
            token={token}
            persona={selected}
            onCopied={(p) => {
              setPersonas((ps) => [...ps, p]);
              setSelectedId(p.id);
            }}
          />
        )}
        {selected && !selected.owner_name && (
          <PersonaEditor
            key={selected.id}
            token={token}
            persona={selected}
            models={models}
            onSaved={(p) => setPersonas((ps) => ps.map((x) => (x.id === p.id ? p : x)))}
            onDeleted={(id) => {
              setPersonas((ps) => ps.filter((x) => x.id !== id));
              setSelectedId(null);
            }}
          />
        )}
      </div>
      <MemoryPanel token={token} personas={personas} />
    </div>
  );
}

function PersonaEditor({
  token,
  persona,
  models,
  onSaved,
  onDeleted,
}: {
  token: string;
  persona: Persona;
  models: InstalledModel[];
  onSaved: (p: Persona) => void;
  onDeleted: (id: string) => void;
}) {
  const [name, setName] = useState(persona.name);
  const [slug, setSlug] = useState(persona.slug);
  const [slugTouched, setSlugTouched] = useState(false);
  // A persona still called "New persona" gets its id from its first real name.
  const placeholder = /^new-persona(-\d+)?$/.test(persona.slug);
  const shownSlug = !slugTouched && placeholder ? toSlug(name) || persona.slug : slug;
  const [prompt, setPrompt] = useState(persona.system_prompt);
  const [sending, setSending] = useState(false);
  const [model, setModel] = useState(persona.model ?? "");
  const [temperature, setTemperature] = useState(persona.temperature ?? 0.7);
  const [useTemp, setUseTemp] = useState(persona.temperature !== null);
  const [think, setThink] = useState(persona.think);
  const [useMemory, setUseMemory] = useState(persona.use_memory);
  const [voice, setVoice] = useState(persona.voice ?? "");
  const [speakReplies, setSpeakReplies] = useState(persona.speak_replies);
  const [shared, setShared] = useState(persona.shared ?? false);
  const [voices, setVoices] = useState<VoiceStatus | null>(null);
  const [trying, setTrying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    voiceApi.status(token).then(setVoices, () => undefined);
  }, [token]);

  async function tryVoice() {
    setTrying(true);
    setError(null);
    try {
      const blob = await voiceApi.speak(token, `Hi, I'm ${name || "your persona"}. This is how I sound.`, {
        voice: voice || voices?.default_voice,
      });
      await new Promise<void>((resolve) => {
        const a = new Audio(URL.createObjectURL(blob));
        a.onended = () => resolve();
        a.onerror = () => resolve();
        void a.play().catch(() => resolve());
      });
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Couldn't play the voice.");
    } finally {
      setTrying(false);
    }
  }
  const [saved, setSaved] = useState(false);

  const thinkingModel = models.find((m) => m.model === model)?.badges.includes("thinking") ?? true;

  async function save(e: FormEvent) {
    e.preventDefault();
    setError(null);
    try {
      const p = await api.updatePersona(token, persona.id, {
        name,
        ...(slugTouched && slug.trim() && slug.trim() !== persona.slug ? { slug: slug.trim() } : {}),
        system_prompt: prompt,
        model,
        ...(useTemp ? { temperature } : {}),
        think,
        use_memory: useMemory,
        shared,
        voice,
        speak_replies: speakReplies,
      });
      onSaved(p);
      setSlug(p.slug);
      setSlugTouched(false);
      setSaved(true);
      window.setTimeout(() => setSaved(false), 2000);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  async function remove() {
    if (!window.confirm(`Delete ${persona.name}? Its chats are kept.`)) return;
    try {
      await api.deletePersona(token, persona.id);
      onDeleted(persona.id);
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  return (
    <form onSubmit={save} className="flex min-w-0 flex-1 flex-col gap-5" noValidate>
      <div className="flex flex-wrap items-center gap-4">
        <Avatar name={name} color={persona.avatar_color} size={56} />
        <div className="flex-1">
          <h2 className="text-xl font-semibold">{persona.name}</h2>
          <p className="text-sm text-muted">
            API model id <code className="font-mono text-soft">persona:{persona.slug}</code>
          </p>
        </div>
        <Button variant="ghost" onClick={() => setSending(true)} title="Give someone their own copy">
          Send a copy
        </Button>
        <Button variant="danger" onClick={() => void remove()}>
          Delete
        </Button>
        <Button type="submit">{saved ? "Saved" : "Save"}</Button>
      </div>
      {sending && <SendCopy token={token} persona={persona} onClose={() => setSending(false)} />}
      <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} maxLength={40} />
      <label className="flex flex-col gap-1.5 text-sm text-muted">
        API model id
        <span className="flex items-center rounded-xl border border-line bg-surface focus-within:border-ember">
          <span className="pl-3.5 font-mono text-[15px] text-muted">persona:</span>
          <input
            value={shownSlug}
            onChange={(e) => {
              setSlugTouched(true);
              setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9-]+/g, "-"));
            }}
            maxLength={40}
            spellCheck={false}
            className="min-h-11 flex-1 bg-transparent pr-3.5 font-mono text-[15px] text-ink focus:outline-none"
          />
        </span>
        <span className="text-xs">
          What Hermes and other apps use as the model. Changing it means updating those apps too.
        </span>
      </label>
      <label className="flex flex-col gap-1.5 text-sm text-muted">
        System prompt
        <textarea
          rows={7}
          value={prompt}
          onChange={(e) => setPrompt(e.target.value)}
          placeholder="Who is this persona? How should it talk? What should it always or never do?"
          className="rounded-xl border border-line bg-surface p-3.5 text-[15px] leading-relaxed text-ink focus:border-ember focus:outline-none"
        />
        <label className="mt-2 flex items-start gap-2 text-sm text-ink">
          <input type="checkbox" className="mt-1 accent-ember" checked={shared} onChange={(e) => setShared(e.target.checked)} />
          <span>
            Share with everyone on this Outpost
            <span className="block text-xs text-muted">
              They can chat with it (their chats and memories stay theirs) and make their own copy. Only you can change it.
            </span>
          </span>
        </label>
        <span className="text-xs">
          <code className="font-mono text-soft">{"{name}"}</code> becomes the name of whoever is chatting, e.g. "I am
          talking with {"{name}"}." That makes one persona (and one description of your family) right for everyone
          you send it to. It works in memories too.
        </span>
      </label>
      <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
        <label className="flex flex-col gap-1.5 text-sm text-muted">
          Model
          <select
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="min-h-11 rounded-xl border border-line bg-surface px-3 text-ink"
          >
            <option value="">Default (first downloaded chat model)</option>
            {models.map((m) => (
              <option key={m.model} value={m.model}>
                {m.model}
              </option>
            ))}
          </select>
        </label>
        <div className="flex flex-col gap-1.5 text-sm text-muted">
          <label className="flex items-center gap-2">
            <input type="checkbox" checked={useTemp} onChange={(e) => setUseTemp(e.target.checked)} className="accent-ember" />
            Set temperature: {useTemp ? temperature.toFixed(1) : "model default"}
          </label>
          <input
            type="range"
            min={0}
            max={2}
            step={0.1}
            value={temperature}
            disabled={!useTemp}
            onChange={(e) => setTemperature(Number(e.target.value))}
            className="accent-ember"
            aria-label="Temperature"
          />
          <span className="text-xs">Lower is focused, higher is creative.</span>
        </div>
      </div>
      <div className="flex flex-col gap-3 text-sm">
        <label className="flex items-center gap-3">
          <input type="checkbox" checked={think} onChange={(e) => setThink(e.target.checked)} className="size-4 accent-ember" />
          <span>
            Show reasoning (thinking models only)
            {!thinkingModel && <span className="text-muted"> · this model doesn't think</span>}
          </span>
        </label>
        <label className="flex items-center gap-3">
          <input
            type="checkbox"
            checked={useMemory}
            onChange={(e) => setUseMemory(e.target.checked)}
            className="size-4 accent-ember"
          />
          Use memory (what you asked it to remember, plus facts learned from chats)
        </label>
      </div>
      <PersonaDocuments token={token} personaId={persona.id} shared={shared} />
      <div className="flex flex-col gap-3 rounded-2xl border border-line p-4 text-sm">
        <span className="font-medium">Voice</span>
        {voices && !voices.ready ? (
          <span className="text-muted">
            Voice isn't running. Install it in Settings › Engines to give personas a voice and talk to them.
          </span>
        ) : (
          <div className="flex flex-wrap items-center gap-3">
            <select
              aria-label="Speaking voice"
              value={voice}
              onChange={(e) => setVoice(e.target.value)}
              className="min-h-11 rounded-xl border border-line bg-deep px-3 text-ink"
            >
              <option value="">Default ({voices?.voices.find((v) => v.id === voices.default_voice)?.name ?? "Heart"})</option>
              {[...new Set((voices?.voices ?? []).map((v) => v.language))].map((lang) => (
                <optgroup key={lang} label={lang}>
                  {(voices?.voices ?? [])
                    .filter((v) => v.language === lang)
                    .map((v) => (
                      <option key={v.id} value={v.id}>
                        {v.name} ({v.gender})
                      </option>
                    ))}
                </optgroup>
              ))}
            </select>
            <Button variant="ghost" disabled={trying || !voices?.ready} onClick={() => void tryVoice()}>
              {trying ? "Playing…" : "Hear it"}
            </Button>
          </div>
        )}
        <label className="flex items-center gap-3">
          <input
            type="checkbox"
            checked={speakReplies}
            onChange={(e) => setSpeakReplies(e.target.checked)}
            className="size-4 accent-ember"
          />
          Read replies aloud in chat
        </label>
      </div>
      <ErrorNote>{error}</ErrorNote>
    </form>
  );
}

/** Someone else's shared persona: what it is, and a button to make my own copy. */
function SharedPersona({ token, persona, onCopied }: { token: string; persona: Persona; onCopied: (p: Persona) => void }) {
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <section className="flex min-w-0 flex-1 flex-col gap-5">
      <div className="flex flex-wrap items-center gap-4">
        <Avatar name={persona.name} color={persona.avatar_color} size={56} />
        <div className="flex-1">
          <h2 className="text-xl font-semibold">{persona.name}</h2>
          <p className="text-sm text-muted">
            Shared by {persona.owner_name}. Chat with it any time; your chats and memories with it are yours.
          </p>
        </div>
        <Button
          disabled={busy}
          onClick={() => {
            setBusy(true);
            setError(null);
            api.copyPersona(token, persona.id).then(onCopied, (e) => {
              setError(e instanceof ApiError ? e.message : "Couldn't copy it.");
              setBusy(false);
            });
          }}
        >
          Make my own copy
        </Button>
      </div>
      <ErrorNote>{error}</ErrorNote>
      <div className="flex flex-col gap-1.5 text-sm text-muted">
        System prompt
        <p className="rounded-xl border border-line bg-surface p-3.5 text-[15px] leading-relaxed whitespace-pre-wrap text-soft">
          {persona.system_prompt || "(none)"}
        </p>
      </div>
      <p className="text-sm text-muted">
        Model: <span className="text-ink">{persona.model ?? "the default model"}</span>
        {persona.voice ? (
          <>
            {" "}· Voice: <span className="text-ink">{persona.voice}</span>
          </>
        ) : null}
      </p>
      <p className="text-xs text-muted">Only {persona.owner_name} can change it. Make your own copy to change yours.</p>
    </section>
  );
}

/** Give other people their own copy of a persona (not my memories). */
function SendCopy({ token, persona, onClose }: { token: string; persona: Persona; onClose: () => void }) {
  const [people, setPeople] = useState<DirectoryEntry[] | null>(null);
  const [chosen, setChosen] = useState<string[]>([]);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    api.people(token).then(
      (all) => setPeople(all.filter((p) => p.id !== persona.account_id)),
      (e) => setError(e instanceof ApiError ? e.message : "Can't load the people list."),
    );
  }, [token, persona.account_id]);

  async function send() {
    setBusy(true);
    setError(null);
    try {
      const sent = await api.sendPersona(token, persona.id, chosen);
      setDone(
        sent.length
          ? `Sent: ${sent.map((s) => (s.persona_name === persona.name ? s.account_name : `${s.account_name} (as “${s.persona_name}”)`)).join(", ")}.`
          : "Nobody was picked.",
      );
    } catch (e) {
      setError(e instanceof ApiError ? e.message : "Couldn't send it.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <div role="dialog" aria-modal="true" aria-label={`Send ${persona.name}`} className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onClick={onClose}>
      <div className="flex w-full max-w-md flex-col gap-4 rounded-2xl border border-line bg-night p-6" onClick={(e) => e.stopPropagation()}>
        <h2 className="text-lg font-semibold">Send a copy of {persona.name}</h2>
        <p className="text-sm text-muted">
          They each get their own copy: its prompt, model, voice and settings, ready to change. Your memories stay
          with you. Save your changes first: the copy is made from what's saved.
        </p>
        {done ? (
          <>
            <p className="text-sm" role="status">
              {done}
            </p>
            <Button onClick={onClose} className="self-start">
              Done
            </Button>
          </>
        ) : (
          <>
            {people === null ? (
              <p className="text-sm text-muted">Loading…</p>
            ) : people.length === 0 ? (
              <p className="text-sm text-muted">There's nobody else on this Outpost yet. Invite someone in Settings › Accounts.</p>
            ) : (
              <ul className="flex flex-col gap-2">
                {people.map((p) => (
                  <li key={p.id}>
                    <label className="flex items-center gap-3 rounded-xl border border-line px-3 py-2 text-sm">
                      <input
                        type="checkbox"
                        className="accent-ember"
                        checked={chosen.includes(p.id)}
                        onChange={(e) => setChosen((c) => (e.target.checked ? [...c, p.id] : c.filter((x) => x !== p.id)))}
                      />
                      <Avatar name={p.name} color={p.avatar_color} size={28} />
                      {p.name}
                    </label>
                  </li>
                ))}
              </ul>
            )}
            <ErrorNote>{error}</ErrorNote>
            <div className="flex gap-3">
              <Button onClick={() => void send()} disabled={busy || chosen.length === 0}>
                {busy ? "Sending…" : "Send"}
              </Button>
              <Button variant="ghost" onClick={onClose}>
                Cancel
              </Button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function MemoryPanel({ token, personas }: { token: string; personas: Persona[] }) {
  const [items, setItems] = useState<Memory[]>([]);
  const [settings, setSettings] = useState<MemorySettings | null>(null);
  const [text, setText] = useState("");
  const [scope, setScope] = useState("");
  const [error, setError] = useState<string | null>(null);

  const fail = (err: unknown) => setError(err instanceof ApiError ? err.message : "Something went wrong.");
  const load = useCallback(() => {
    api.memory(token).then(setItems, (e) => setError(e instanceof ApiError ? e.message : "Can't load memory."));
    api.memorySettings(token).then(setSettings, () => undefined);
  }, [token]);
  useEffect(load, [load]);

  async function add(e: FormEvent) {
    e.preventDefault();
    if (!text.trim()) return;
    try {
      await api.addMemory(token, text, scope || undefined);
      setText("");
      load();
    } catch (err) {
      fail(err);
    }
  }

  async function forget(id: string) {
    try {
      await api.deleteMemory(token, id);
      setItems((m) => m.filter((x) => x.id !== id));
    } catch (err) {
      fail(err);
    }
  }

  async function toggleAuto() {
    if (!settings) return;
    try {
      setSettings(await api.setMemorySettings(token, !settings.auto));
    } catch (err) {
      fail(err);
    }
  }

  async function clearLearned() {
    if (!window.confirm("Forget everything Outpost learned from your chats? Things you added yourself stay.")) return;
    try {
      await api.clearAutoMemory(token);
      load();
    } catch (err) {
      fail(err);
    }
  }

  const manual = items.filter((m) => m.source !== "auto");
  const learned = items.filter((m) => m.source === "auto");
  const scopeLabel = (m: Memory) =>
    m.persona_id ? `${personas.find((p) => p.id === m.persona_id)?.name ?? "one persona"} only` : "all personas";

  return (
    <section aria-label="Memory" className="flex flex-col gap-5 rounded-2xl border border-line bg-surface p-5">
      <div>
        <h2 className="text-lg font-semibold">Memory</h2>
        <p className="text-sm text-muted">
          What personas with memory on know about you. Private to your account and stored only on this PC.
        </p>
      </div>

      <div className="flex flex-col gap-3">
        <h3 className="text-sm font-semibold text-soft">Always remember</h3>
        <form onSubmit={add} className="flex flex-wrap items-end gap-3">
          <div className="min-w-64 flex-1">
            <Field label="Remember" value={text} onChange={(e) => setText(e.target.value)} placeholder="I live in Sydney" maxLength={500} />
          </div>
          <label className="flex flex-col gap-1.5 text-sm text-muted">
            For
            <select value={scope} onChange={(e) => setScope(e.target.value)} className="min-h-11 rounded-xl border border-line bg-deep px-3 text-ink">
              <option value="">All personas</option>
              {personas.map((p) => (
                <option key={p.id} value={p.id}>
                  {p.name} only
                </option>
              ))}
            </select>
          </label>
          <Button type="submit" disabled={!text.trim()}>
            Remember
          </Button>
        </form>
        {manual.length > 0 && (
          <ul className="flex flex-col gap-2">
            {manual.map((m) => (
              <MemoryRow key={m.id} memory={m} scope={scopeLabel(m)} onForget={() => void forget(m.id)} />
            ))}
          </ul>
        )}
      </div>

      <div className="flex flex-col gap-3">
        <div className="flex flex-wrap items-center gap-3">
          <h3 className="flex-1 text-sm font-semibold text-soft">Learned from chats</h3>
          {learned.length > 0 && (
            <Button variant="ghost" onClick={() => void clearLearned()}>
              Forget all learned
            </Button>
          )}
          <Button variant={settings?.auto ? "ghost" : "primary"} disabled={!settings} onClick={() => void toggleAuto()}>
            {settings?.auto ? "Turn off learning" : "Turn on learning"}
          </Button>
        </div>
        <p className="text-sm text-muted">
          {settings?.auto === false
            ? "Learning is off: nothing new is picked up from your chats."
            : "After each reply, the chat's model notes lasting facts you mention (your name, pets, projects, preferences). Each persona keeps what it learned, and only the facts that fit the conversation are used. Deleting a chat forgets what was learned in it."}
          {settings && !settings.embedding_model && settings.auto && (
            <>
              {" "}
              For smarter recall, download <b>Qwen3 Embedding 0.6B</b> in Models; until then the newest facts are used.
            </>
          )}
        </p>
        {learned.length > 0 ? (
          <ul className="flex flex-col gap-2">
            {learned.map((m) => (
              <MemoryRow key={m.id} memory={m} scope={m.persona_id ? `learned by ${scopeLabel(m).replace(" only", "")}` : "learned"} onForget={() => void forget(m.id)} />
            ))}
          </ul>
        ) : (
          <p className="text-sm text-muted/80">Nothing learned yet.</p>
        )}
      </div>
      <ErrorNote>{error}</ErrorNote>
    </section>
  );
}

function MemoryRow({ memory, scope, onForget }: { memory: Memory; scope: string; onForget: () => void }) {
  return (
    <li className="flex items-center gap-3 rounded-xl bg-raised px-4 py-2.5 text-sm">
      <span className="flex-1">{memory.text}</span>
      <span className="text-xs text-muted">{scope}</span>
      <button type="button" onClick={onForget} className="text-xs text-muted hover:text-danger">
        Forget
      </button>
    </li>
  );
}

/** Same idea as the gateway's slugify: lowercase words joined by dashes. */
function toSlug(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 40);
}

/** Documents the persona knows, in every chat with it. Saved straight away. */
function PersonaDocuments({ token, personaId, shared }: { token: string; personaId: string; shared: boolean }) {
  const [docs, setDocs] = useState<Doc[]>([]);
  const [picking, setPicking] = useState(false);
  useEffect(() => {
    docsApi.forPersona(token, personaId).then(setDocs, () => setDocs([]));
  }, [token, personaId]);
  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-line p-4 text-sm">
      <span className="font-medium">Documents</span>
      <span className="text-muted">
        {docs.length
          ? `Knows ${docs.map((d) => d.name).join(", ")} in every chat.`
          : "Give it documents (a manual, notes, a handbook) and it answers from them in every chat."}
        {shared && docs.length > 0 ? " It's shared, so everyone who uses it can ask about these." : ""}
      </span>
      <Button variant="ghost" className="self-start" onClick={() => setPicking(true)}>
        {docs.length ? "Change documents" : "Choose documents"}
      </Button>
      {picking && (
        <DocumentPicker
          token={token}
          title="Documents for this persona"
          note={
            shared
              ? "This persona is shared: everyone who chats with it can ask about the documents you tick (they can't download them)."
              : "Tick the documents it should know in every chat."
          }
          chosen={docs.map((d) => d.id)}
          onSave={async (ids) => setDocs(await docsApi.setForPersona(token, personaId, ids))}
          onClose={() => setPicking(false)}
        />
      )}
    </div>
  );
}

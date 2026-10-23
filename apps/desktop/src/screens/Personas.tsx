import { useCallback, useEffect, useState, type FormEvent } from "react";
import { api, ApiError, type InstalledModel, type Memory, type Persona } from "../api";
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
                <span className="truncate font-mono text-xs text-muted">persona:{p.slug}</span>
                <span className="truncate text-xs text-muted">{p.model ?? "default model"}</span>
              </span>
            </button>
          ))}
        </section>

        {selected && (
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
  const [model, setModel] = useState(persona.model ?? "");
  const [temperature, setTemperature] = useState(persona.temperature ?? 0.7);
  const [useTemp, setUseTemp] = useState(persona.temperature !== null);
  const [think, setThink] = useState(persona.think);
  const [useMemory, setUseMemory] = useState(persona.use_memory);
  const [error, setError] = useState<string | null>(null);
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
        <Button variant="danger" onClick={() => void remove()}>
          Delete
        </Button>
        <Button type="submit">{saved ? "Saved" : "Save"}</Button>
      </div>
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
          Use memory (things you asked Outpost to remember)
        </label>
      </div>
      <ErrorNote>{error}</ErrorNote>
    </form>
  );
}

function MemoryPanel({ token, personas }: { token: string; personas: Persona[] }) {
  const [items, setItems] = useState<Memory[]>([]);
  const [text, setText] = useState("");
  const [scope, setScope] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    api.memory(token).then(setItems, (e) => setError(e instanceof ApiError ? e.message : "Can't load memory."));
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
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  async function forget(id: string) {
    try {
      await api.deleteMemory(token, id);
      setItems((m) => m.filter((x) => x.id !== id));
    } catch (err) {
      setError(err instanceof ApiError ? err.message : "Something went wrong.");
    }
  }

  return (
    <section aria-label="Memory" className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
      <div>
        <h2 className="text-lg font-semibold">Memory</h2>
        <p className="text-sm text-muted">
          Facts every persona with memory on will know, like your name or where you live. Private to your account.
        </p>
      </div>
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
      <ErrorNote>{error}</ErrorNote>
      {items.length > 0 && (
        <ul className="flex flex-col gap-2">
          {items.map((m) => (
            <li key={m.id} className="flex items-center gap-3 rounded-xl bg-raised px-4 py-2.5 text-sm">
              <span className="flex-1">{m.text}</span>
              <span className="text-xs text-muted">
                {m.persona_id ? `${personas.find((p) => p.id === m.persona_id)?.name ?? "one persona"} only` : "all personas"}
              </span>
              <button type="button" onClick={() => void forget(m.id)} className="text-xs text-muted hover:text-danger">
                Forget
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
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

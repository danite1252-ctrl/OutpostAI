import { useCallback, useEffect, useRef, useState, type DragEvent } from "react";
import {
  ApiError,
  createApi,
  trainApi,
  type Character,
  type Dataset,
  type DatasetImage,
  type DatasetView,
  type LoraFile,
  type SignInResponse,
  type TrainingStatus,
  type TrainingView,
} from "../api";
import { formatEta } from "../components/downloads";
import { Lightbox, useAuthedUrl } from "../components/images";
import { jobsChanged, progressFraction } from "../components/queue";
import { Button, ErrorNote, Field, Icon, icons } from "../components/ui";

function message(err: unknown) {
  return err instanceof ApiError ? err.message : "Something went wrong.";
}

/** Seconds per step on a 16 GB card with Outpost's settings, from the spike (≈4.5 s). */
const SECS_PER_STEP = 4.5;
const STEP_CHOICES = [1000, 1500, 2000, 3000];

const ACCEPTED = ["image/png", "image/jpeg", "image/webp"];
const MAX_BYTES = 40 * 1024 * 1024;

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(`${file.name} couldn't be read.`));
    reader.readAsDataURL(file);
  });
}

/** Training Studio: pictures of a character, captions, a training, then pick the best checkpoint. */
export default function Train({ auth, onUnauthorized }: { auth: SignInResponse; onUnauthorized: () => void }) {
  const token = auth.token;
  const [status, setStatus] = useState<TrainingStatus | null>(null);
  const [datasets, setDatasets] = useState<Dataset[]>([]);
  const [characters, setCharacters] = useState<Character[]>([]);
  const [selected, setSelected] = useState<string | null>(null);
  const [creating, setCreating] = useState(false);
  const [importing, setImporting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fail = useCallback(
    (err: unknown) => {
      if (err instanceof ApiError && err.status === 401) return onUnauthorized();
      setError(message(err));
    },
    [onUnauthorized],
  );
  const loadList = useCallback(() => {
    trainApi.datasets(token).then(setDatasets, fail);
    trainApi.characters(token).then(setCharacters, () => undefined);
  }, [token, fail]);
  const loadStatus = useCallback(() => trainApi.status(token).then(setStatus, () => undefined), [token]);
  useEffect(() => {
    loadList();
    void loadStatus();
  }, [loadList, loadStatus]);
  useEffect(() => {
    if (!selected && datasets.length > 0) setSelected(datasets[0].id);
  }, [datasets, selected]);

  return (
    <section className="mx-auto flex max-w-6xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold">Train</h1>
        <p className="text-sm text-muted">
          Teach Outpost a character from 15 to 30 pictures. It trains overnight on your graphics card, then you pick the
          best result and use it by name in Create and chat.
        </p>
      </div>
      <SetupNote status={status} isOwner={auth.account.role === "owner"} token={token} onSaved={setStatus} />
      <ErrorNote>{error}</ErrorNote>
      <div className="flex flex-col gap-6 lg:flex-row">
        <aside className="flex w-full shrink-0 flex-col gap-4 lg:w-64">
          <Button onClick={() => setCreating(true)}>New character</Button>
          {datasets.length > 0 && (
            <nav aria-label="Datasets" className="flex flex-col gap-1">
              <h2 className="px-1 text-xs font-semibold tracking-wide text-muted uppercase">Datasets</h2>
              {datasets.map((d) => (
                <button
                  key={d.id}
                  type="button"
                  onClick={() => setSelected(d.id)}
                  aria-current={selected === d.id ? "true" : undefined}
                  className={`flex flex-col rounded-xl px-3 py-2 text-left ${selected === d.id ? "bg-raised" : "hover:bg-raised/60"}`}
                >
                  <span className="truncate font-medium">{d.name}</span>
                  <span className="text-xs text-muted">
                    <span className="font-mono">{d.trigger_word}</span> · {d.image_count} picture{d.image_count === 1 ? "" : "s"}
                  </span>
                </button>
              ))}
            </nav>
          )}
          <CharacterList token={token} characters={characters} onChanged={loadList} onError={fail} onImport={() => setImporting(true)} />
        </aside>
        <div className="min-w-0 flex-1">
          {selected ? (
            <DatasetPanel
              key={selected}
              token={token}
              id={selected}
              status={status}
              onChanged={loadList}
              onDeleted={() => {
                setSelected(null);
                loadList();
              }}
              onError={fail}
            />
          ) : (
            <Intro onStart={() => setCreating(true)} />
          )}
        </div>
      </div>
      {importing && (
        <ImportCharacter
          token={token}
          onClose={() => setImporting(false)}
          onAdded={() => {
            setImporting(false);
            loadList();
          }}
        />
      )}
      {creating && (
        <NewDataset
          token={token}
          onClose={() => setCreating(false)}
          onMade={(d) => {
            setCreating(false);
            loadList();
            setSelected(d.id);
          }}
        />
      )}
    </section>
  );
}

function Intro({ onStart }: { onStart: () => void }) {
  return (
    <div className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-6">
      <h2 className="text-lg font-semibold">How it works</h2>
      <ol className="flex list-decimal flex-col gap-2 pl-5 text-sm">
        <li>Make a character and add 15 to 30 clear pictures of them: different outfits, places and angles, mostly the face and upper body.</li>
        <li>Outpost writes a caption for each with a vision model. Fix any that are wrong.</li>
        <li>Start the training. It takes the graphics card for two to three hours; chat waits until it's done.</li>
        <li>Compare the test pictures from each checkpoint and keep the one that looks most like them.</li>
        <li>Use them by name: Create › Characters, or "draw Kat at the beach" in chat.</li>
      </ol>
      <Button className="self-start" onClick={onStart}>
        New character
      </Button>
    </div>
  );
}

function SetupNote({
  status,
  isOwner,
  token,
  onSaved,
}: {
  status: TrainingStatus | null;
  isOwner: boolean;
  token: string;
  onSaved: (s: TrainingStatus) => void;
}) {
  const [dir, setDir] = useState("");
  const [error, setError] = useState<string | null>(null);
  useEffect(() => setDir(status?.toolkit_dir ?? ""), [status?.toolkit_dir]);
  if (!status) return null;
  const notes: string[] = [];
  if (status.caption_models.length === 0) notes.push("No model that can see pictures is installed, so captions must be written by hand. Get one with the eye badge in Models.");
  if (!status.character_pack_ready) notes.push("Z-Image Turbo isn't downloaded yet: get it in Create › Characters to use a trained character.");
  if (status.vram_gb > 0 && status.vram_gb < status.vram_needed_gb)
    notes.push(`Training needs about ${status.vram_needed_gb} GB of video memory; this PC has ${status.vram_gb.toFixed(0)} GB, so it may run out.`);
  if (!status.toolkit_problem && notes.length === 0) return null;
  return (
    <div className="flex flex-col gap-3 rounded-2xl border border-line bg-surface p-5" role="status">
      {status.toolkit_problem && (
        <>
          <p className="text-sm">
            <b>Training isn't set up yet.</b> {status.toolkit_problem}
          </p>
          {isOwner && (
            <form
              className="flex flex-wrap items-end gap-3"
              onSubmit={(e) => {
                e.preventDefault();
                setError(null);
                trainApi.setToolkitDir(token, dir).then(onSaved, (err: unknown) => setError(message(err)));
              }}
            >
              <div className="min-w-72 flex-1">
                <Field label="ai-toolkit folder" value={dir} onChange={(e) => setDir(e.target.value)} placeholder="C:\dev\ai-toolkit" />
              </div>
              <Button type="submit" variant="ghost">
                Save
              </Button>
            </form>
          )}
          <ErrorNote>{error}</ErrorNote>
        </>
      )}
      {notes.map((n) => (
        <p key={n} className="text-sm text-muted">
          {n}
        </p>
      ))}
    </div>
  );
}

function NewDataset({ token, onClose, onMade }: { token: string; onClose: () => void; onMade: (d: DatasetView) => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div role="dialog" aria-modal="true" aria-label="New character" className="fixed inset-0 z-40 flex items-center justify-center bg-black/80 p-6" onClick={onClose}>
      <form
        className="flex w-full max-w-md flex-col gap-4 rounded-2xl border border-line bg-surface p-6"
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          trainApi.createDataset(token, name).then(onMade, (err: unknown) => {
            setError(message(err));
            setBusy(false);
          });
        }}
      >
        <h2 className="text-xl font-semibold">New character</h2>
        <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Kat" autoFocus hint="What you'll call them in chat. Outpost suggests a trigger word next." />
        <ErrorNote>{error}</ErrorNote>
        <div className="flex gap-3">
          <Button type="submit" disabled={busy || !name.trim()}>
            {busy ? "Making…" : "Make"}
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        </div>
      </form>
    </div>
  );
}

function DatasetPanel({
  token,
  id,
  status,
  onChanged,
  onDeleted,
  onError,
}: {
  token: string;
  id: string;
  status: TrainingStatus | null;
  onChanged: () => void;
  onDeleted: () => void;
  onError: (e: unknown) => void;
}) {
  const [view, setView] = useState<DatasetView | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => trainApi.dataset(token, id).then(setView, onError), [token, id, onError]);
  useEffect(() => void load(), [load]);

  const latest = view?.trainings[0] ?? null;
  const active = !!view?.captioning?.running || (latest != null && (latest.status === "queued" || latest.status === "running"));
  useEffect(() => {
    if (!active) return;
    const t = window.setInterval(() => void load(), latest?.status === "running" ? 3000 : 2000);
    return () => window.clearInterval(t);
  }, [active, load, latest?.status]);
  // Captions finishing changes the counts in the list too.
  const wasActive = useRef(false);
  useEffect(() => {
    if (wasActive.current && !active) onChanged();
    wasActive.current = active;
  }, [active, onChanged]);

  if (!view) return <div className="h-64 animate-pulse rounded-2xl bg-surface" />;
  const fail = (err: unknown) => setError(message(err));
  const locked = latest != null && (latest.status === "queued" || latest.status === "running");

  async function remove() {
    if (!view || !window.confirm(`Delete ${view.name}'s dataset and its training runs? Characters already made from it stay.`)) return;
    try {
      await trainApi.deleteDataset(token, view.id);
      onDeleted();
    } catch (err) {
      fail(err);
    }
  }

  return (
    <div className="flex flex-col gap-8">
      <Header token={token} view={view} locked={locked} onSaved={(v) => {
          setView(v);
          onChanged();
        }} onDelete={() => void remove()} />
      <ErrorNote>{error}</ErrorNote>
      <Pictures token={token} view={view} locked={locked} status={status} onChanged={() => void load().then(onChanged)} onError={fail} />
      <TrainBox token={token} view={view} status={status} locked={locked} onQueued={() => void load()} onError={fail} />
      {view.trainings.map((t, i) => (
        <TrainingRun key={t.id} token={token} training={t} name={view.name} latest={i === 0} onChanged={() => {
            void load();
            onChanged();
          }} />
      ))}
    </div>
  );
}

function Header({
  token,
  view,
  locked,
  onSaved,
  onDelete,
}: {
  token: string;
  view: DatasetView;
  locked: boolean;
  onSaved: (v: DatasetView) => void;
  onDelete: () => void;
}) {
  const [name, setName] = useState(view.name);
  const [word, setWord] = useState(view.trigger_word);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setName(view.name);
    setWord(view.trigger_word);
  }, [view.name, view.trigger_word]);
  const check = view.trigger_check;
  const save = (body: { name?: string; trigger_word?: string }) => {
    setError(null);
    trainApi.updateDataset(token, view.id, body).then(onSaved, (err: unknown) => {
      setError(message(err));
      setName(view.name);
      setWord(view.trigger_word);
    });
  };
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-end gap-4">
        <div className="min-w-48 flex-1">
          <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} onBlur={() => name.trim() !== view.name && save({ name })} />
        </div>
        <div className="w-56">
          <Field
            label="Trigger word"
            value={word}
            disabled={locked}
            onChange={(e) => setWord(e.target.value.toLowerCase().replace(/[^a-z0-9]/g, ""))}
            onBlur={() => word !== view.trigger_word && save({ trigger_word: word })}
            className="min-h-11 rounded-xl border border-line bg-surface px-3.5 font-mono text-base text-ink focus:border-ember focus:outline-none"
          />
        </div>
        <Button variant="danger" onClick={onDelete} disabled={locked}>
          Delete
        </Button>
      </div>
      {check.ok ? (
        <p className="text-xs text-muted">
          <span className="text-ok">✓</span> <span className="font-mono">{view.trigger_word}</span> is a made-up word the model doesn't know yet:
          good. Prompts start with it.
        </p>
      ) : (
        <div className="flex flex-wrap items-center gap-3 rounded-xl border border-warn/60 bg-warn/10 px-4 py-3 text-sm" role="alert">
          <span className="flex-1">
            <b>Pick a made-up trigger word.</b> {check.problem} A real word fights what the model already knows, so the character comes out
            weaker.
          </span>
          {check.suggestion && !locked && (
            <Button variant="ghost" onClick={() => save({ trigger_word: check.suggestion! })}>
              Use <span className="font-mono">{check.suggestion}</span>
            </Button>
          )}
        </div>
      )}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

function Pictures({
  token,
  view,
  locked,
  status,
  onChanged,
  onError,
}: {
  token: string;
  view: DatasetView;
  locked: boolean;
  status: TrainingStatus | null;
  onChanged: () => void;
  onError: (e: unknown) => void;
}) {
  const [adding, setAdding] = useState<{ done: number; total: number } | null>(null);
  const [folder, setFolder] = useState("");
  const [note, setNote] = useState<string | null>(null);
  const [model, setModel] = useState("");
  const [dragging, setDragging] = useState(false);
  const picker = useRef<HTMLInputElement | null>(null);
  const models = status?.caption_models ?? [];
  const cap = view.captioning;

  async function addFiles(files: File[]) {
    const ok = files.filter((f) => ACCEPTED.includes(f.type));
    const problems = files.filter((f) => !ACCEPTED.includes(f.type)).map((f) => `${f.name}: not a PNG, JPEG or WebP picture`);
    setAdding({ done: 0, total: ok.length });
    for (const [n, f] of ok.entries()) {
      try {
        if (f.size > MAX_BYTES) throw new Error(`${f.name} is over 40 MB.`);
        await trainApi.addImage(token, view.id, await readAsDataUrl(f));
      } catch (err) {
        problems.push(err instanceof Error ? err.message : `${f.name} couldn't be added.`);
      }
      setAdding({ done: n + 1, total: ok.length });
    }
    setAdding(null);
    setNote(problems.length ? `Left out: ${problems.join("; ")}` : null);
    onChanged();
  }

  async function importFolder() {
    setNote(null);
    try {
      const r = await trainApi.importFolder(token, view.id, folder);
      setNote(`Added ${r.added} picture${r.added === 1 ? "" : "s"}.${r.skipped.length ? ` Left out: ${r.skipped.join("; ")}` : ""}`);
      setFolder("");
      onChanged();
    } catch (err) {
      onError(err);
    }
  }

  async function caption(overwrite: boolean) {
    try {
      await trainApi.caption(token, view.id, { model: model || undefined, overwrite });
      onChanged();
    } catch (err) {
      onError(err);
    }
  }

  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    setDragging(false);
    if (!locked) void addFiles(Array.from(e.dataTransfer.files));
  };
  const missing = view.image_count - view.captioned_count;

  return (
    <div className="flex flex-col gap-4">
      <div className="flex flex-wrap items-end gap-3">
        <div className="flex-1">
          <h2 className="text-lg font-semibold">Pictures</h2>
          <p className="text-sm text-muted">
            {view.image_count} added · {view.captioned_count} captioned. 15 to 30 is best: clear, varied, one person, no other faces.
          </p>
        </div>
        <Button variant="ghost" disabled={locked || adding != null} onClick={() => picker.current?.click()}>
          Add pictures…
        </Button>
        <input
          ref={picker}
          type="file"
          accept="image/png,image/jpeg,image/webp"
          multiple
          className="hidden"
          onChange={(e) => {
            const files = Array.from(e.target.files ?? []);
            e.target.value = "";
            void addFiles(files);
          }}
        />
      </div>
      <form
        className="flex flex-wrap items-end gap-3"
        onSubmit={(e) => {
          e.preventDefault();
          void importFolder();
        }}
      >
        <div className="min-w-72 flex-1">
          <Field
            label="Or copy a folder from this PC"
            value={folder}
            disabled={locked}
            onChange={(e) => setFolder(e.target.value)}
            placeholder="C:\dev\dataset\kat"
            hint="Captions in .txt files of the same name come too."
          />
        </div>
        <Button type="submit" variant="ghost" disabled={locked || !folder.trim()}>
          Copy pictures
        </Button>
      </form>
      {adding && (
        <p className="text-sm text-muted" role="status">
          Adding {adding.done} of {adding.total}…
        </p>
      )}
      {note && <p className="text-sm text-muted">{note}</p>}

      <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-line bg-surface p-4">
        <span className="flex flex-1 flex-col text-sm">
          <b>Captions</b>
          <span className="text-muted">
            {cap?.running
              ? `Writing captions with ${cap.model}: ${cap.done + cap.failed} of ${cap.total}…`
              : cap && cap.total > 0
                ? `Last run: ${cap.done} written${cap.failed ? `, ${cap.failed} failed (${cap.error ?? "unknown"})` : ""}.`
                : "A vision model describes each picture; the trigger word is added in front when training."}
          </span>
        </span>
        {models.length > 1 && (
          <select
            aria-label="Caption model"
            value={model}
            onChange={(e) => setModel(e.target.value)}
            className="min-h-11 rounded-xl border border-line bg-deep px-3 text-sm text-ink"
          >
            <option value="">{models[0]}</option>
            {models.slice(1).map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        )}
        <Button disabled={locked || cap?.running || models.length === 0 || missing === 0} onClick={() => void caption(false)}>
          Caption {missing > 0 ? missing : ""} missing
        </Button>
        <Button variant="ghost" disabled={locked || cap?.running || models.length === 0 || view.image_count === 0} onClick={() => {
          if (window.confirm("Write new captions for every picture, replacing the ones there?")) void caption(true);
        }}>
          Redo all
        </Button>
      </div>

      <ul
        onDragOver={(e) => {
          e.preventDefault();
          setDragging(true);
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
        className={`grid min-h-32 grid-cols-1 gap-3 rounded-2xl sm:grid-cols-2 xl:grid-cols-3 ${dragging ? "outline-2 outline-ember outline-dashed" : ""}`}
      >
        {view.images.length === 0 && (
          <li className="col-span-full flex min-h-32 items-center justify-center rounded-2xl border border-dashed border-line text-sm text-muted">
            Drop pictures here
          </li>
        )}
        {view.images.map((img) => (
          <PictureCard key={img.id} token={token} datasetId={view.id} image={img} locked={locked} onChanged={onChanged} onError={onError} />
        ))}
      </ul>
    </div>
  );
}

function PictureCard({
  token,
  datasetId,
  image,
  locked,
  onChanged,
  onError,
}: {
  token: string;
  datasetId: string;
  image: DatasetImage;
  locked: boolean;
  onChanged: () => void;
  onError: (e: unknown) => void;
}) {
  const { src } = useAuthedUrl(token, trainApi.imagePath(datasetId, image.id));
  const [text, setText] = useState(image.caption);
  const [open, setOpen] = useState(false);
  useEffect(() => setText(image.caption), [image.caption]);
  return (
    <li className="flex gap-3 rounded-2xl border border-line bg-surface p-3">
      <button type="button" onClick={() => setOpen(true)} className="size-24 shrink-0 overflow-hidden rounded-xl bg-raised" aria-label="View larger">
        {src && <img src={src} alt="" className="h-full w-full object-cover" loading="lazy" />}
      </button>
      <div className="flex min-w-0 flex-1 flex-col gap-1.5">
        <textarea
          aria-label="Caption"
          value={text}
          disabled={locked}
          rows={3}
          placeholder="No caption yet"
          onChange={(e) => setText(e.target.value)}
          onBlur={() => {
            if (text.trim() === image.caption) return;
            trainApi.setCaption(token, datasetId, image.id, text).then(onChanged, onError);
          }}
          className="w-full flex-1 resize-none rounded-lg border border-line bg-deep px-2 py-1.5 text-xs leading-relaxed text-ink focus:border-ember focus:outline-none"
        />
        <button
          type="button"
          disabled={locked}
          onClick={() => trainApi.deleteImage(token, datasetId, image.id).then(onChanged, onError)}
          className="self-end text-xs text-muted hover:text-danger disabled:opacity-40"
        >
          Remove
        </button>
      </div>
      {open && src && <Lightbox src={src} alt={image.caption || "Training picture"} onClose={() => setOpen(false)} />}
    </li>
  );
}

function TrainBox({
  token,
  view,
  status,
  locked,
  onQueued,
  onError,
}: {
  token: string;
  view: DatasetView;
  status: TrainingStatus | null;
  locked: boolean;
  onQueued: () => void;
  onError: (e: unknown) => void;
}) {
  const [steps, setSteps] = useState(2000);
  const [busy, setBusy] = useState(false);
  // Test prompts: Outpost's suggestion from the captions until you change one.
  const [prompts, setPrompts] = useState<string[]>(view.suggested_prompts);
  const [edited, setEdited] = useState(false);
  const suggestionKey = view.suggested_prompts.join("\n");
  useEffect(() => {
    if (!edited) setPrompts(suggestionKey ? suggestionKey.split("\n") : []);
  }, [suggestionKey, edited]);
  if (locked) return null;
  const hours = (steps * SECS_PER_STEP) / 3600;
  const blockers = [
    status?.toolkit_problem ? "ai-toolkit isn't set up" : null,
    view.image_count < 5 ? "add at least 5 pictures" : null,
    view.captioning?.running ? "captions are being written" : null,
  ].filter(Boolean);
  async function start() {
    if (!view.trigger_check.ok && !window.confirm(`"${view.trigger_word}" may not train well (${view.trigger_check.problem}). Train anyway?`)) return;
    setBusy(true);
    try {
      await trainApi.train(token, view.id, steps, prompts.filter((p) => p.trim()));
      jobsChanged();
      onQueued();
    } catch (err) {
      onError(err);
    } finally {
      setBusy(false);
    }
  }
  return (
    <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-line bg-surface p-5">
      <span className="flex flex-1 flex-col gap-1 text-sm">
        <b>Train {view.name}</b>
        <span className="text-muted">
          About {hours < 1 ? `${Math.round(hours * 60)} minutes` : `${hours.toFixed(1)} hours`} on the graphics card, with test pictures every 250
          steps. Chat pauses while it trains; leave the PC on.
          {view.captioned_count < view.image_count && view.image_count > 0 && " Pictures without a caption train on the trigger word alone."}
        </span>
        {blockers.length > 0 && <span className="text-warn">First: {blockers.join(", ")}.</span>}
      </span>
      <select
        aria-label="Steps"
        value={steps}
        onChange={(e) => setSteps(Number(e.target.value))}
        className="min-h-11 rounded-xl border border-line bg-deep px-3 text-sm text-ink"
      >
        {STEP_CHOICES.map((s) => (
          <option key={s} value={s}>
            {s} steps{s === 2000 ? " (usual)" : ""}
          </option>
        ))}
      </select>
      <Button disabled={busy || blockers.length > 0} onClick={() => void start()}>
        {busy ? "Queuing…" : "Start training"}
      </Button>
      <details className="w-full">
        <summary className="cursor-pointer text-sm text-muted">
          Test pictures: {prompts.length} prompt{prompts.length === 1 ? "" : "s"} drawn every 250 steps
        </summary>
        <div className="mt-3 flex flex-col gap-2">
          <p className="text-xs text-muted">
            Made from your captions, so they match what you're training (a person, a dog, a cartoon style). Change them to
            see the character in the scenes you care about. Each starts with the trigger word.
          </p>
          {prompts.map((p, i) => (
            <input
              key={i}
              aria-label={`Test prompt ${i + 1}`}
              value={p}
              onChange={(e) => {
                setEdited(true);
                setPrompts((cur) => cur.map((x, j) => (j === i ? e.target.value : x)));
              }}
              className="min-h-10 rounded-xl border border-line bg-deep px-3 text-sm text-ink focus:border-ember focus:outline-none"
            />
          ))}
          {edited && (
            <button
              type="button"
              className="self-start text-xs text-ember hover:text-ember-hover"
              onClick={() => {
                setEdited(false);
                setPrompts(view.suggested_prompts);
              }}
            >
              Back to the suggested prompts
            </button>
          )}
        </div>
      </details>
    </div>
  );
}

function TrainingRun({
  token,
  training: t,
  name,
  latest,
  onChanged,
}: {
  token: string;
  training: TrainingView;
  name: string;
  latest: boolean;
  onChanged: () => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [busyStep, setBusyStep] = useState<number | null>(null);
  const f = progressFraction(t.progress);
  const when = new Date(t.created_at * 1000).toLocaleString();
  const header = {
    queued: t.position ? `Waiting: ${t.position} job${t.position === 1 ? "" : "s"} ahead` : "Next up",
    running: t.progress?.phase ?? "Training",
    done: "Finished",
    failed: "Failed",
    cancelled: "Stopped",
  }[t.status];

  async function use(step: number) {
    setBusyStep(step);
    setError(null);
    try {
      await trainApi.useCheckpoint(token, t.id, step);
      onChanged();
    } catch (err) {
      setError(message(err));
    } finally {
      setBusyStep(null);
    }
  }

  async function stop() {
    if (!window.confirm("Stop this training? Checkpoints saved so far stay and can still be used.")) return;
    await createApi.cancelJob(token, t.job_id).catch(() => undefined);
    jobsChanged();
    onChanged();
  }

  const body = (
    <div className="flex flex-col gap-4">
      {t.status === "running" && (
        <div className="flex flex-col gap-1.5" aria-live="polite">
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            <div className="h-2 rounded-full bg-ember transition-all" style={{ width: `${(f ?? 0) * 100}%` }} />
          </div>
          <span className="text-xs text-muted">
            {t.progress && t.progress.steps > 0 ? `Step ${t.progress.step} of ${t.progress.steps}` : t.progress?.phase}
            {f != null && ` · ${Math.round(f * 100)}%`}
            {t.eta_secs != null && ` · ${formatEta(t.eta_secs)}`}
          </span>
        </div>
      )}
      {t.error && <ErrorNote>{t.error}</ErrorNote>}
      <ErrorNote>{error}</ErrorNote>
      {t.checkpoints.length > 0 ? (
        <div className="flex flex-col gap-3">
          <p className="text-sm text-muted">
            Each row is the same test scenes at a point in the training. Early ones look less like {name}; late ones can look
            stiff or over-baked. Keep the one that looks most like them and still follows the prompt.
          </p>
          {t.sample_prompts.length > 0 && (
            <ol className="flex list-decimal flex-col gap-0.5 pl-5 text-xs text-muted">
              {t.sample_prompts.map((p) => (
                <li key={p}>{p}</li>
              ))}
            </ol>
          )}
          <ol className="flex flex-col gap-3">
            {[...t.checkpoints].reverse().map((c) => (
              <li key={c.step} className="flex flex-wrap items-center gap-3 rounded-2xl border border-line bg-surface p-3">
                <span className="w-24 shrink-0 text-sm">
                  <b>{c.step === 0 ? "Before" : `Step ${c.step}`}</b>
                  {t.chosen_step === c.step && <span className="block text-xs text-ok">✓ In use</span>}
                </span>
                <span className="flex flex-1 flex-wrap gap-2">
                  {c.samples.map((s, i) => (
                    <Sample key={s} token={token} trainingId={t.id} file={s} prompt={t.sample_prompts[i]} />
                  ))}
                </span>
                {c.file && (
                  <Button
                    variant={t.chosen_step === c.step ? "ghost" : "primary"}
                    disabled={busyStep != null || t.status === "running" || t.status === "queued"}
                    onClick={() => void use(c.step)}
                  >
                    {busyStep === c.step ? "Copying…" : t.chosen_step === c.step ? "Use again" : "Use this one"}
                  </Button>
                )}
              </li>
            ))}
          </ol>
        </div>
      ) : (
        t.status !== "failed" && t.status !== "cancelled" && <p className="text-sm text-muted">Test pictures appear here every 250 steps.</p>
      )}
    </div>
  );

  return (
    <section className="flex flex-col gap-3" aria-label={`Training from ${when}`}>
      <div className="flex flex-wrap items-center gap-3">
        <h2 className="flex-1 text-lg font-semibold">
          {header} <span className="text-sm font-normal text-muted">· {t.steps} steps, {t.image_count} pictures · {when}</span>
        </h2>
        {(t.status === "running" || t.status === "queued") && (
          <Button variant="danger" onClick={() => void stop()}>
            {t.status === "running" ? "Stop" : "Cancel"}
          </Button>
        )}
      </div>
      {latest ? body : <details><summary className="cursor-pointer text-sm text-muted">Show checkpoints</summary><div className="mt-3">{body}</div></details>}
    </section>
  );
}

function Sample({ token, trainingId, file, prompt }: { token: string; trainingId: string; file: string; prompt?: string }) {
  const { src, failed } = useAuthedUrl(token, trainApi.samplePath(trainingId, file));
  const [open, setOpen] = useState(false);
  const box = "size-36 overflow-hidden rounded-xl border border-line bg-raised";
  if (failed) return <span className={`${box} flex items-center justify-center text-xs text-muted`}>Missing</span>;
  return (
    <>
      <button
        type="button"
        className={`${box} hover:border-ember`}
        onClick={() => src && setOpen(true)}
        title={prompt}
        aria-label={prompt ? `View test picture: ${prompt}` : "View test picture"}
      >
        {src ? <img src={src} alt="" className="h-full w-full object-cover" loading="lazy" /> : <span className="block h-full w-full animate-pulse" />}
      </button>
      {open && src && <Lightbox src={src} alt={prompt ?? "Test picture"} onClose={() => setOpen(false)} />}
    </>
  );
}

function CharacterList({
  token,
  characters,
  onChanged,
  onError,
  onImport,
}: {
  token: string;
  characters: Character[];
  onChanged: () => void;
  onError: (e: unknown) => void;
  onImport: () => void;
}) {
  async function remove(c: Character) {
    if (!window.confirm(`Remove ${c.name}? ${c.training_id ? "Its LoRA file is deleted too; the dataset stays, so you can pick a checkpoint again." : ""}`)) return;
    await trainApi.deleteCharacter(token, c.id).then(onChanged, onError);
  }
  return (
    <div className="flex flex-col gap-1">
      <h2 className="px-1 text-xs font-semibold tracking-wide text-muted uppercase">Characters</h2>
      {characters.length === 0 && <p className="px-1 text-xs text-muted">None yet.</p>}
      {characters.map((c) => (
        <div key={c.id} className="group flex items-center gap-2 rounded-xl px-3 py-2 hover:bg-raised/60">
          <span className="flex min-w-0 flex-1 flex-col">
            <span className="truncate font-medium">{c.name}</span>
            <span className="text-xs text-muted">
              <span className="font-mono">{c.trigger_word}</span>
              {c.step != null && ` · step ${c.step}`}
            </span>
          </span>
          <button
            type="button"
            onClick={() => void remove(c)}
            aria-label={`Remove ${c.name}`}
            className="flex size-8 items-center justify-center rounded-lg text-muted opacity-0 group-hover:opacity-100 hover:text-danger focus:opacity-100"
          >
            <Icon d={icons.close} size={14} />
          </button>
        </div>
      ))}
      <button type="button" onClick={onImport} className="self-start px-1 pt-1 text-xs text-ember hover:text-ember-hover">
        Add from a LoRA file…
      </button>
    </div>
  );
}

/** Make a character from a LoRA trained elsewhere, already in the LoRA folder. */
function ImportCharacter({ token, onClose, onAdded }: { token: string; onClose: () => void; onAdded: () => void }) {
  const [files, setFiles] = useState<LoraFile[] | null>(null);
  const [file, setFile] = useState("");
  const [name, setName] = useState("");
  const [trigger, setTrigger] = useState("");
  const [subject, setSubject] = useState("");
  const [style, setStyle] = useState("photo");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    trainApi.loraFiles(token).then(setFiles, (err: unknown) => setError(message(err)));
  }, [token]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  const usable = (files ?? []).filter((f) => f.z_image && !f.character);
  const other = (files ?? []).filter((f) => !f.z_image || f.character);
  const picked = files?.find((f) => f.file === file) ?? null;

  function pick(f: string) {
    setFile(f);
    const info = files?.find((x) => x.file === f);
    // Fill in what the file knows; the name is a guess to edit.
    if (info?.trigger) setTrigger(info.trigger);
    if (!name.trim() && info) {
      const guess = (info.trigger ?? info.file.replace(/\.safetensors$/i, ""))
        .replace(/^char_|_?z_?image$/gi, "")
        .replace(/zzz_?/gi, "")
        .replace(/[_-]+/g, " ")
        .trim();
      if (guess) setName(guess.charAt(0).toUpperCase() + guess.slice(1));
    }
  }

  async function add() {
    setBusy(true);
    setError(null);
    try {
      await trainApi.addCharacter(token, { lora_file: file, name, trigger_word: trigger, subject, style });
      onAdded();
    } catch (err) {
      setError(message(err));
      setBusy(false);
    }
  }

  const box = "min-h-11 rounded-xl border border-line bg-deep px-3 text-ink focus:border-ember focus:outline-none";
  return (
    <div role="dialog" aria-modal="true" aria-label="Add a character from a LoRA file" className="fixed inset-0 z-40 flex items-center justify-center bg-black/80 p-6" onClick={onClose}>
      <form
        className="flex max-h-full w-full max-w-lg flex-col gap-4 overflow-auto rounded-2xl border border-line bg-surface p-6"
        onClick={(e) => e.stopPropagation()}
        onSubmit={(e) => {
          e.preventDefault();
          void add();
        }}
      >
        <h2 className="text-xl font-semibold">Add a character from a LoRA file</h2>
        <p className="text-sm text-muted">
          For a Z-Image LoRA trained somewhere else. Copy its <span className="font-mono">.safetensors</span> file into the LoRA folder
          first, then pick it here.
        </p>
        <label className="flex flex-col gap-1.5 text-sm text-muted">
          LoRA file
          <select value={file} onChange={(e) => pick(e.target.value)} className={box}>
            <option value="">{files == null ? "Looking…" : usable.length ? "Pick one" : "No new Z-Image LoRAs in the folder"}</option>
            {usable.map((f) => (
              <option key={f.file} value={f.file}>
                {f.file.replace(/\.safetensors$/i, "")}
              </option>
            ))}
            {other.length > 0 && (
              <optgroup label="Can't be added">
                {other.map((f) => (
                  <option key={f.file} value={f.file} disabled>
                    {f.file.replace(/\.safetensors$/i, "")} ({f.character ? `already ${f.character}` : "not for Z-Image"})
                  </option>
                ))}
              </optgroup>
            )}
          </select>
        </label>
        {picked && (
          <p className="text-xs text-muted">
            {picked.trigger
              ? `It was trained with the trigger word "${picked.trigger}".`
              : "It doesn't say its trigger word: use the word it was trained with."}
            {picked.trained_as ? ` Saved as "${picked.trained_as}".` : ""}
          </p>
        )}
        <Field label="Name" value={name} onChange={(e) => setName(e.target.value)} placeholder="Kat" hint="What you'll call them in chat." />
        <Field label="Trigger word" value={trigger} onChange={(e) => setTrigger(e.target.value)} placeholder="zk4t" />
        <div className="flex gap-3">
          <div className="flex-1">
            <Field label="What they are" value={subject} onChange={(e) => setSubject(e.target.value)} placeholder="woman, man, dog…" />
          </div>
          <div className="flex-1">
            <Field label="Picture style" value={style} onChange={(e) => setStyle(e.target.value)} placeholder="photo, cartoon style…" />
          </div>
        </div>
        <p className="text-xs text-muted">
          Chat puts these beside the trigger word ("zk4t woman at the beach, photo"), so the model knows what to draw.
        </p>
        <ErrorNote>{error}</ErrorNote>
        <div className="flex gap-3">
          <Button type="submit" disabled={busy || !file || !name.trim() || !trigger.trim()}>
            {busy ? "Adding…" : "Add character"}
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
        </div>
      </form>
    </div>
  );
}

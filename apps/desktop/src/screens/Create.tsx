import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { api, ApiError, createApi, type Character, type EngineInfo, type Job, type Media, type Pack, type PackInput, type Task } from "../api";
import { downloadsChanged, formatBytes, formatEta, formatSpeed } from "../components/downloads";
import { GalleryGrid, GalleryPicker, mediaAsInput, Viewer, type CreateRequest } from "../components/gallery";
import { dropCached, prepareImage } from "../components/images";
import { jobsChanged } from "../components/queue";
import { Button, ErrorNote } from "../components/ui";

const TASKS: { id: Task; label: string; blurb: string }[] = [
  { id: "picture", label: "Text to image", blurb: "Describe a picture" },
  { id: "character", label: "Characters", blurb: "Your trained characters" },
  { id: "edit", label: "Edit", blurb: "Change a picture" },
  { id: "video", label: "Video", blurb: "Text to video" },
  { id: "animate", label: "Animate", blurb: "Bring a picture to life" },
];

/** How a workflow fits this PC, in the same colours as the Models page. */
function fitBadge(p: Pack) {
  if (p.missing_nodes.length > 0) return { text: "Needs a newer ComfyUI", style: "text-muted", dot: "bg-line" };
  if (p.fit === "too_large") return { text: "Too big for your machine", style: "text-danger", dot: "bg-danger" };
  if (p.fit === "partial") return { text: "Bigger than your card: runs slowly", style: "text-warn", dot: "bg-warn" };
  if (p.recommended) return { text: "Best for this PC", style: "text-pick", dot: "bg-pick" };
  return { text: "Fits your PC", style: "text-ok", dot: "bg-ok" };
}

const CHOICE_KEY = "outpost.create.choice";

function loadChoices(): Partial<Record<Task, string>> {
  try {
    return JSON.parse(localStorage.getItem(CHOICE_KEY) ?? "{}") as Partial<Record<Task, string>>;
  } catch {
    return {};
  }
}

function saveChoices(c: Partial<Record<Task, string>>) {
  try {
    localStorage.setItem(CHOICE_KEY, JSON.stringify(c));
  } catch {
    // Not remembered: Outpost's pick is used next time.
  }
}

type Values = Record<string, unknown>;

/** LoRA files, and which of them are my trained characters. */
type LoraInfo = { loras: string[]; folder: string; characters: Character[] };

function message(err: unknown) {
  return err instanceof ApiError ? err.message : "Something went wrong.";
}

function defaults(pack: Pack): Values {
  const v: Values = {};
  for (const i of pack.inputs) v[i.key] = i.default ?? (i.kind === "text" ? "" : null);
  return v;
}

/** A task's workflows, Outpost's pick first, then best first; ones that can't run last. */
function packsOf(packs: Pack[], task: Task) {
  const order = (p: Pack) => (p.fit === "too_large" || p.missing_nodes.length ? 2 : p.fit === "partial" ? 1 : 0);
  return packs
    .filter((p) => p.task === task)
    .sort((a, b) => Number(b.recommended) - Number(a.recommended) || order(a) - order(b) || b.rank - a.rank);
}

export default function Create({
  token,
  canManage,
  comfyInstalled,
  comfy,
  onEnginesChanged,
  request,
  onRequestHandled,
  onOpenGallery,
  onUnauthorized,
}: {
  token: string;
  canManage: boolean;
  comfyInstalled: boolean;
  /** ComfyUI's engine card, for its version and the update button. */
  comfy: EngineInfo | null;
  onEnginesChanged: () => void;
  /** Something the Gallery asked for: repeat a picture, or use one as an input. */
  request: CreateRequest | null;
  onRequestHandled: () => void;
  onOpenGallery: () => void;
  onUnauthorized: () => void;
}) {
  const [packs, setPacks] = useState<Pack[]>([]);
  const [task, setTask] = useState<Task>("picture");
  const [choices, setChoices] = useState<Partial<Record<Task, string>>>(loadChoices);
  const [values, setValues] = useState<Values>({});
  const [loras, setLoras] = useState<LoraInfo>({ loras: [], folder: "", characters: [] });
  const [jobs, setJobs] = useState<Job[]>([]);
  const [gallery, setGallery] = useState<Media[]>([]);
  const [viewing, setViewing] = useState<Media | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const finishedSeen = useRef<Set<string>>(new Set());
  // Values to apply once a given pack is showing ("Use these settings", "Edit this picture").
  const pending = useRef<{ pack?: string; task: Task; values?: Values; image?: string } | null>(null);

  const fail = useCallback(
    (err: unknown) => {
      if (err instanceof ApiError && err.status === 401) return onUnauthorized();
      setError(message(err));
    },
    [onUnauthorized],
  );

  const loadPacks = useCallback(() => createApi.packs(token).then(setPacks, fail), [token, fail]);
  const loadLoras = useCallback(() => createApi.loras(token).then(setLoras, () => undefined), [token]);
  const loadGallery = useCallback(() => createApi.gallery(token).then(setGallery, fail), [token, fail]);
  const loadJobs = useCallback(
    () =>
      createApi.jobs(token).then((list) => {
        setJobs(list);
        // A job just finished: show what it made.
        let fresh = false;
        for (const j of list) {
          if (j.status === "done" && !finishedSeen.current.has(j.id)) {
            finishedSeen.current.add(j.id);
            fresh = true;
          }
        }
        if (fresh) void loadGallery();
      }, fail),
    [token, fail, loadGallery],
  );

  useEffect(() => {
    void loadPacks();
    void loadLoras();
    void loadGallery();
    void createApi.jobs(token).then((list) => {
      for (const j of list) if (j.status === "done") finishedSeen.current.add(j.id);
      setJobs(list);
    }, fail);
  }, [token, loadPacks, loadLoras, loadGallery, fail]);

  const options = useMemo(() => packsOf(packs, task), [packs, task]);
  const pack = options.find((p) => p.id === choices[task]) ?? options[0] ?? null;

  function choose(p: Pack) {
    const next = { ...choices, [p.task]: p.id };
    setChoices(next);
    saveChoices(next);
  }

  useEffect(() => {
    if (!pack) return;
    const want = pending.current;
    if (want && want.task === pack.task && (!want.pack || want.pack === pack.id)) {
      pending.current = null;
      const v: Values = { ...defaults(pack), ...pickKnown(values, pack), ...(want.values ?? {}) };
      const imageInput = pack.inputs.find((i) => i.kind === "image");
      if (want.image && imageInput) v[imageInput.key] = want.image;
      setValues(v);
    } else {
      setValues((v) => ({ ...defaults(pack), ...pickKnown(v, pack) }));
    }
  }, [pack?.id]); // eslint-disable-line react-hooks/exhaustive-deps

  // Requests from the Gallery screen.
  useEffect(() => {
    if (!request || packs.length === 0) return;
    onRequestHandled();
    void handle(request);
  }, [request, packs.length]); // eslint-disable-line react-hooks/exhaustive-deps

  async function handle(r: CreateRequest) {
    setError(null);
    if (r.kind === "reuse") return reuse(r.media);
    try {
      const image = await mediaAsInput(token, r.media);
      pending.current = { task: r.task, image };
      if (task === r.task && pack) {
        // Already showing: apply now.
        const imageInput = pack.inputs.find((i) => i.kind === "image");
        pending.current = null;
        if (imageInput) setValues((v) => ({ ...v, [imageInput.key]: image }));
      } else {
        setTask(r.task);
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "That picture couldn't be used.");
    }
  }

  // Poll while something is moving: jobs queued or running, or models downloading.
  const active = jobs.some((j) => j.status === "queued" || j.status === "running");
  const downloading = packs.some((p) => p.models.setup?.running);
  useEffect(() => {
    if (!active && !downloading) return;
    const t = window.setInterval(() => {
      if (active) void loadJobs();
      if (downloading) void loadPacks();
    }, 1500);
    return () => window.clearInterval(t);
  }, [active, downloading, loadJobs, loadPacks]);

  async function generate(e?: FormEvent) {
    e?.preventDefault();
    if (!pack || busy) return;
    setBusy(true);
    setError(null);
    try {
      const inputs: Values = {};
      for (const i of pack.inputs) {
        const v = values[i.key];
        if (v !== null && v !== undefined && v !== "") inputs[i.key] = v;
      }
      const job = await createApi.createJob(token, pack.id, inputs);
      setJobs((list) => [job, ...list]);
      jobsChanged();
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  }

  async function install(p: Pack) {
    setError(null);
    if (
      p.fit === "partial" &&
      !window.confirm(
        `${p.name} needs about ${p.vram_gb} GB of video memory, more than your graphics card has. It will run, but slowly. Download it anyway?`,
      )
    )
      return;
    try {
      await createApi.installPack(token, p.id);
      downloadsChanged();
      await loadPacks();
    } catch (err) {
      fail(err);
    }
  }

  async function updateComfy() {
    if (
      !window.confirm(
        "Update ComfyUI now? It stops for a few minutes while the new version installs, and a running job is stopped. Your models and gallery stay.",
      )
    )
      return;
    setError(null);
    try {
      await api.engineAction(token, "comfyui", "install");
      onEnginesChanged();
    } catch (err) {
      fail(err);
    }
  }

  async function cancel(j: Job) {
    try {
      await createApi.cancelJob(token, j.id);
      jobsChanged();
      await loadJobs();
    } catch (err) {
      fail(err);
    }
  }

  async function remove(m: Media) {
    if (!window.confirm("Delete this from your gallery?")) return;
    try {
      await createApi.deleteMedia(token, m.id);
      dropCached(`/api/media/${m.id}`);
      setGallery((g) => g.filter((x) => x.id !== m.id));
      setViewing(null);
    } catch (err) {
      fail(err);
    }
  }

  async function share(m: Media, shared: boolean) {
    try {
      const updated = await createApi.shareMedia(token, m.id, shared);
      setGallery((g) => g.map((x) => (x.id === m.id ? updated : x)));
      setViewing(updated);
    } catch (err) {
      fail(err);
    }
  }

  /** Fill the form with everything a picture was made with (pictures it was given aren't kept). */
  async function reuse(m: Media) {
    setViewing(null);
    let restored: Values = { prompt: m.prompt ?? "", seed: m.seed ?? null };
    if (m.job_id) {
      try {
        const job = await createApi.job(token, m.job_id);
        const inputs = JSON.parse(job.inputs) as Values;
        for (const [k, v] of Object.entries(inputs)) {
          if (v && typeof v === "object") delete inputs[k]; // stored input pictures
        }
        restored = { ...inputs, seed: m.seed ?? null };
      } catch {
        // Older item or job gone: prompt and seed are enough.
      }
    }
    const target = packs.find((p) => p.id === m.pack_id) ?? pack;
    if (!target) return;
    if (target.id === pack?.id) {
      setValues((v) => ({ ...v, ...restored }));
      return;
    }
    pending.current = { pack: target.id, task: target.task, values: restored };
    // Show the workflow that made it, for this once (the saved choice stays).
    setChoices((c) => ({ ...c, [target.task]: target.id }));
    setTask(target.task);
  }

  const queue = jobs.filter((j) => j.status === "queued" || j.status === "running");
  const recentFailures = jobs.filter((j) => j.status === "failed").slice(0, 3);

  return (
    <section className="mx-auto flex max-w-6xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold">Create</h1>
        <p className="text-sm text-muted">
          Make pictures and videos on this PC. Jobs wait their turn for the graphics card, so chat may pause while one runs.
        </p>
      </div>
      {!comfyInstalled && (
        <p className="rounded-2xl border border-warn/40 bg-surface p-4 text-sm" role="status">
          Create runs on ComfyUI. Install it in <b>Settings › Engines</b> first; Outpost starts it when a job needs it.
        </p>
      )}
      <ErrorNote>{error}</ErrorNote>
      {comfy?.managed && comfy.version && comfy.version !== comfy.pinned_version && comfy.state !== "installing" && (
        <div className="flex flex-wrap items-center gap-3 rounded-2xl border border-pick/40 bg-surface p-4 text-sm" role="status">
          <span className="flex-1">
            ComfyUI {comfy.pinned_version} is available (you have {comfy.version}). Newer workflows may need it.
          </span>
          {canManage && <Button onClick={() => void updateComfy()}>Update ComfyUI</Button>}
        </div>
      )}
      {comfy?.state === "installing" && (
        <p className="rounded-2xl border border-line bg-surface p-4 text-sm" role="status">
          ComfyUI is updating. Workflows come back when it's done (see Settings › Engines for progress).
        </p>
      )}

      <div className="flex flex-wrap gap-2" role="tablist" aria-label="What to make">
        {TASKS.map((t) => (
          <button
            key={t.id}
            type="button"
            role="tab"
            aria-selected={t.id === task}
            onClick={() => setTask(t.id)}
            className={`flex min-h-11 flex-col items-start rounded-xl border px-4 py-2 text-left ${
              t.id === task ? "border-ember bg-raised" : "border-line bg-surface hover:bg-raised"
            }`}
          >
            <span className="text-sm font-semibold">{t.label}</span>
            <span className="text-xs text-muted">{t.blurb}</span>
          </button>
        ))}
      </div>

      {pack && (
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
          <div className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
            <ModelPicker options={options} current={pack} onChoose={choose} />
            <div>
              <p className="text-sm text-soft">{pack.description}</p>
              <p className={`mt-1 text-xs ${fitBadge(pack).style}`}>
                {fitBadge(pack).text} · needs about {pack.vram_gb} GB of video memory
                {pack.license ? ` · ${pack.license} licence` : ""}
              </p>
            </div>
            {pack.missing_nodes.length > 0 ? (
              <NeedsNewerComfy
                pack={pack}
                comfy={comfy}
                canManage={canManage}
                onUpdate={() => void updateComfy()}
              />
            ) : pack.models.ready ? (
              <PackForm
                token={token}
                pack={pack}
                values={values}
                loras={loras}
                busy={busy}
                onChange={(key, v) => setValues((s) => ({ ...s, [key]: v }))}
                onRefreshLoras={() => void loadLoras()}
                onSubmit={generate}
              />
            ) : (
              <ModelSetup pack={pack} canManage={canManage} onInstall={() => void install(pack)} />
            )}
          </div>

          <div className="flex flex-col gap-4">
            <h2 className="text-lg font-semibold">Queue</h2>
            {queue.length === 0 && recentFailures.length === 0 && (
              <p className="text-sm text-muted">Nothing waiting. New pictures appear in the gallery below.</p>
            )}
            <ul className="flex flex-col gap-2">
              {queue.map((j) => (
                <JobRow key={j.id} job={j} packs={packs} onCancel={() => void cancel(j)} />
              ))}
              {recentFailures.map((j) => (
                <JobRow key={j.id} job={j} packs={packs} />
              ))}
            </ul>
          </div>
        </div>
      )}

      <div className="flex flex-col gap-3">
        <div className="flex items-center gap-3">
          <h2 className="flex-1 text-lg font-semibold">Recent</h2>
          {gallery.length > 0 && (
            <Button variant="ghost" onClick={onOpenGallery}>
              Open the gallery
            </Button>
          )}
        </div>
        {gallery.length === 0 ? (
          <p className="text-sm text-muted">Your pictures will appear here. They're private to your account.</p>
        ) : (
          <GalleryGrid token={token} items={gallery.slice(0, 10)} onOpen={setViewing} />
        )}
      </div>

      {viewing && (
        <Viewer
          token={token}
          media={viewing}
          packName={packs.find((p) => p.id === viewing.pack_id)?.name}
          onClose={() => setViewing(null)}
          onRequest={(r) => {
            setViewing(null);
            void handle(r);
          }}
          onShare={(shared) => void share(viewing, shared)}
          onDelete={() => void remove(viewing)}
        />
      )}
    </section>
  );
}

/** Keep values whose keys the new pack also has: prompts and chosen pictures carry over. */
function pickKnown(v: Values, pack: Pack): Values {
  const out: Values = {};
  for (const i of pack.inputs) if (i.key in v && (i.kind === "text" || i.kind === "image") && v[i.key]) out[i.key] = v[i.key];
  return out;
}

/** A workflow this ComfyUI can't run: say why, and offer the update when Outpost manages ComfyUI. */
function NeedsNewerComfy({
  pack,
  comfy,
  canManage,
  onUpdate,
}: {
  pack: Pack;
  comfy: EngineInfo | null;
  canManage: boolean;
  onUpdate: () => void;
}) {
  const managed = comfy?.managed ?? false;
  return (
    <div className="flex flex-col gap-3 rounded-xl border border-line bg-night p-4 text-sm">
      <p>
        <b>{pack.name}</b> needs a newer ComfyUI{comfy?.version ? ` than ${comfy.version}` : ""}. It's missing{" "}
        <span className="font-mono text-xs">{pack.missing_nodes.join(", ")}</span>.
      </p>
      {managed ? (
        canManage ? (
          <>
            <p className="text-muted">
              Updating installs ComfyUI {comfy?.pinned_version} in a few minutes. Models and the gallery stay.
            </p>
            <Button onClick={onUpdate} className="self-start">
              {comfy?.version === comfy?.pinned_version ? "Reinstall ComfyUI" : "Update ComfyUI"}
            </Button>
          </>
        ) : (
          <p className="text-muted">Ask the owner to update ComfyUI.</p>
        )
      ) : (
        <p className="text-muted">
          This ComfyUI is one you installed yourself, so Outpost can't update it. Update it (for example with ComfyUI
          Manager or <span className="font-mono">git pull</span>), restart it, then reopen Create. Or pick another model
          above.
        </p>
      )}
    </div>
  );
}

/** The task's workflows, coloured by how they fit; Outpost's pick is marked. */
function ModelPicker({ options, current, onChoose }: { options: Pack[]; current: Pack; onChoose: (p: Pack) => void }) {
  if (options.length <= 1) return <h2 className="text-lg font-semibold">{current.name}</h2>;
  return (
    <div className="flex flex-col gap-1.5">
      <span id="model-picker" className="text-sm text-muted">
        Model
      </span>
      <div className="flex flex-wrap gap-2" role="radiogroup" aria-labelledby="model-picker">
        {options.map((p) => {
          const badge = fitBadge(p);
          const blocked = p.fit === "too_large";
          return (
            <button
              key={p.id}
              type="button"
              role="radio"
              aria-checked={p.id === current.id}
              disabled={blocked}
              onClick={() => onChoose(p)}
              title={badge.text}
              className={`flex min-h-11 items-center gap-2 rounded-xl border px-3 py-1.5 text-left ${
                p.id === current.id ? "border-ember bg-raised" : "border-line hover:bg-raised"
              } ${blocked ? "cursor-not-allowed opacity-45" : ""}`}
            >
              <span className={`size-2.5 shrink-0 rounded-full ${badge.dot}`} aria-hidden="true" />
              <span className="flex flex-col leading-tight">
                <span className="text-sm font-semibold">{p.name}</span>
                <span className={`text-xs ${badge.style}`}>
                  {badge.text}
                  {p.models.ready ? "" : ` · ${p.models.missing_gb.toFixed(1)} GB to download`}
                </span>
              </span>
            </button>
          );
        })}
      </div>
    </div>
  );
}

function ModelSetup({ pack, canManage, onInstall }: { pack: Pack; canManage: boolean; onInstall: () => void }) {
  const s = pack.models.setup;
  const running = s?.running ?? false;
  const line = s
    ? [
        s.file_count > 1 ? `File ${Math.max(1, s.file_index)} of ${s.file_count}` : null,
        `${formatBytes(s.done_bytes)} of ${formatBytes(s.total_bytes)}`,
        s.speed_bps ? formatSpeed(s.speed_bps) : null,
        s.eta_secs != null ? formatEta(s.eta_secs) : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "";
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm">
        This workflow needs <b>{pack.models.missing_gb.toFixed(1)} GB</b> of model files before its first picture. They
        download once and stay in Outpost's models folder; files another workflow already has aren't fetched again.
      </p>
      <ul className="flex flex-col gap-1 text-sm">
        {pack.models.files.map((f) => (
          <li key={f.file} className="flex items-center gap-2">
            <span className={f.present ? "text-ok" : "text-muted"}>{f.present ? "✓" : "○"}</span>
            <span className="font-mono text-xs break-all">{f.file}</span>
            <span className="shrink-0 text-xs text-muted">· {f.size_gb} GB</span>
          </li>
        ))}
      </ul>
      {running && s && (
        <div className="flex flex-col gap-1.5" aria-live="polite">
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            <div className="h-2 rounded-full bg-pick transition-all" style={{ width: `${s.percent ?? 0}%` }} />
          </div>
          <span className="text-xs text-muted">{s.current ? `Downloading ${s.current}` : "Starting"}</span>
          <span className="text-xs text-soft">{line}</span>
        </div>
      )}
      {s?.error && !running && <p className="text-sm text-danger">{s.error}</p>}
      {canManage ? (
        !running && (
          <Button onClick={onInstall} disabled={pack.fit === "too_large"}>
            {pack.fit === "too_large"
              ? "Too big for your machine"
              : s?.error
                ? "Try again"
                : `Download ${pack.models.missing_gb.toFixed(1)} GB`}
          </Button>
        )
      ) : (
        <p className="text-sm text-muted">Ask the owner to download these models.</p>
      )}
    </div>
  );
}

function PackForm({
  token,
  pack,
  values,
  loras,
  busy,
  onChange,
  onRefreshLoras,
  onSubmit,
}: {
  token: string;
  pack: Pack;
  values: Values;
  loras: LoraInfo;
  busy: boolean;
  onChange: (key: string, v: unknown) => void;
  onRefreshLoras: () => void;
  onSubmit: (e?: FormEvent) => void;
}) {
  const basic = pack.inputs.filter((i) => !i.advanced);
  const advanced = pack.inputs.filter((i) => i.advanced);
  const missingRequired = pack.inputs.some((i) => i.required && !values[i.key]);
  // For a character workflow, the prompt's hint names the character picked.
  const loraInput = pack.inputs.find((i) => i.kind === "lora");
  const character = loraInput ? loras.characters.find((c) => c.lora_file === values[loraInput.key]) : undefined;
  const promptHelp = !loraInput
    ? undefined
    : character
      ? `Start with ${character.name}'s trigger word and what they are, e.g. "${character.trigger_word}${
          character.subject ? ` ${character.subject}` : ""
        }, at the beach at sunset, ${character.style ?? "photo"}". In chat: "draw ${character.name} at the beach".`
      : "Pick a character above, then start the prompt with its trigger word.";
  const field = (i: PackInput) => (
    <InputField
      key={i.key}
      token={token}
      input={i}
      help={i.key === "prompt" ? promptHelp : undefined}
      value={values[i.key]}
      loras={loras}
      onChange={onChange}
      onRefreshLoras={onRefreshLoras}
      onSubmit={onSubmit}
    />
  );
  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-4" noValidate>
      {basic.map(field)}
      {advanced.length > 0 && (
        <details className="flex flex-col gap-3">
          <summary className="cursor-pointer text-sm text-muted">More settings</summary>
          <div className="mt-3 flex flex-col gap-4">{advanced.map(field)}</div>
        </details>
      )}
      <div className="flex items-center gap-3">
        <Button type="submit" disabled={busy || missingRequired}>
          {busy ? "Adding…" : "Generate"}
        </Button>
        <span className="text-xs text-muted">Ctrl+Enter in the prompt also works.</span>
      </div>
    </form>
  );
}

function InputField({
  token,
  input: i,
  help: helpText,
  value,
  loras,
  onChange,
  onRefreshLoras,
  onSubmit,
}: {
  token: string;
  input: PackInput;
  /** Replaces the input's own hint. */
  help?: string;
  value: unknown;
  loras: LoraInfo;
  onChange: (key: string, v: unknown) => void;
  onRefreshLoras: () => void;
  onSubmit: () => void;
}) {
  const id = `in-${i.key}`;
  const label = (
    <label htmlFor={id} className="text-sm text-muted">
      {i.label}
      {i.required && <span className="text-ember"> *</span>}
    </label>
  );
  const help = (helpText ?? i.help) && <span className="text-xs text-muted">{helpText ?? i.help}</span>;
  const box = "min-h-11 rounded-xl border border-line bg-deep px-3 text-ink focus:border-ember focus:outline-none";

  switch (i.kind) {
    case "text":
      return (
        <div className="flex flex-col gap-1.5">
          {label}
          <textarea
            id={id}
            rows={4}
            value={(value as string) ?? ""}
            onChange={(e) => onChange(i.key, e.target.value)}
            onKeyDown={(e: KeyboardEvent<HTMLTextAreaElement>) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
                e.preventDefault();
                onSubmit();
              }
            }}
            className={`${box} py-2.5 leading-relaxed`}
            maxLength={4000}
          />
          {help}
        </div>
      );
    case "size":
    case "choice":
      return (
        <div className="flex flex-col gap-1.5">
          {label}
          <div className="flex flex-wrap gap-2" role="radiogroup" aria-labelledby={id}>
            {i.options.map((o) => (
              <button
                key={o}
                type="button"
                role="radio"
                aria-checked={value === o}
                onClick={() => onChange(i.key, o)}
                className={`min-h-9 rounded-lg border px-3 font-mono text-xs ${
                  value === o ? "border-ember bg-raised text-ink" : "border-line text-muted hover:text-ink"
                }`}
              >
                {i.kind === "size" ? sizeLabel(o) : o}
              </button>
            ))}
          </div>
          {help}
        </div>
      );
    case "integer":
    case "number":
      return (
        <div className="flex flex-col gap-1.5">
          {label}
          <div className="flex items-center gap-3">
            <input
              id={id}
              type="range"
              min={i.min ?? 0}
              max={i.max ?? 100}
              step={i.step ?? (i.kind === "integer" ? 1 : 0.05)}
              value={Number(value ?? i.default ?? 0)}
              onChange={(e) => onChange(i.key, Number(e.target.value))}
              className="flex-1 accent-ember"
            />
            <span className="w-12 text-right font-mono text-sm">{Number(value ?? i.default ?? 0)}</span>
          </div>
          {help}
        </div>
      );
    case "seed":
      return (
        <div className="flex flex-col gap-1.5">
          {label}
          <input
            id={id}
            inputMode="numeric"
            placeholder="Random"
            value={value === null || value === undefined ? "" : String(value)}
            onChange={(e) => {
              // 15 digits at most, so the seed stays exact (below 2^53).
              const t = e.target.value.replace(/\D/g, "").slice(0, 15);
              onChange(i.key, t ? Number(t) : null);
            }}
            className={`${box} font-mono text-sm`}
          />
          {help}
        </div>
      );
    case "lora":
      return (
        <div className="flex flex-col gap-1.5">
          {label}
          <div className="flex gap-2">
            <select
              id={id}
              value={(value as string) ?? ""}
              onChange={(e) => onChange(i.key, e.target.value || null)}
              className={`${box} flex-1`}
            >
              <option value="">None</option>
              {loras.characters.length > 0 && (
                <optgroup label="My characters">
                  {loras.characters
                    .filter((c) => loras.loras.includes(c.lora_file))
                    .map((c) => (
                      <option key={c.id} value={c.lora_file}>
                        {c.name} ({c.trigger_word})
                      </option>
                    ))}
                </optgroup>
              )}
              <optgroup label="LoRA files">
                {loras.loras
                  .filter((l) => !loras.characters.some((c) => c.lora_file === l))
                  .map((l) => (
                    <option key={l} value={l}>
                      {l.replace(/\.safetensors$/i, "")}
                    </option>
                  ))}
              </optgroup>
            </select>
            <Button variant="ghost" onClick={onRefreshLoras} title="Look for new LoRA files">
              Refresh
            </Button>
          </div>
          <span className="text-xs text-muted">
            {i.help} Train one in Train, or copy its <span className="font-mono">.safetensors</span> file into{" "}
            <span className="font-mono break-all">{loras.folder || "the LoRA folder"}</span>, then Refresh. Start the
            prompt with its trigger word.
          </span>
        </div>
      );
    case "image":
      return (
        <ImageField token={token} id={id} label={label} help={help} value={value as string | null} onPick={(v) => onChange(i.key, v)} />
      );
  }
}

/** Pick, paste or drop a picture; it is sent with the job (large photos are shrunk). */
function ImageField({
  token,
  id,
  label,
  help,
  value,
  onPick,
}: {
  token: string;
  id: string;
  label: ReactNode;
  help: ReactNode;
  value: string | null;
  onPick: (dataUrl: string | null) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [over, setOver] = useState(false);
  const [choosing, setChoosing] = useState(false);
  const picker = useRef<HTMLInputElement | null>(null);
  async function take(file: File | undefined) {
    if (!file) return;
    setError(null);
    try {
      onPick((await prepareImage(file)).dataUrl);
    } catch (e) {
      setError(e instanceof Error ? e.message : "That picture couldn't be used.");
    }
  }
  return (
    <div className="flex flex-col gap-1.5">
      {label}
      <div
        onDragOver={(e) => {
          e.preventDefault();
          setOver(true);
        }}
        onDragLeave={() => setOver(false)}
        onDrop={(e) => {
          e.preventDefault();
          setOver(false);
          void take(e.dataTransfer.files[0]);
        }}
        onPaste={(e) => void take(Array.from(e.clipboardData.files)[0])}
        className={`flex min-h-32 items-center gap-4 rounded-xl border border-dashed p-3 ${over ? "border-ember" : "border-line"}`}
      >
        {value ? (
          <img src={value} alt="Chosen picture" className="h-28 max-w-[50%] rounded-lg object-contain" />
        ) : (
          <span className="flex-1 text-sm text-muted">Drop a picture here, paste one, or choose one.</span>
        )}
        <div className="flex flex-col gap-2">
          <input
            id={id}
            ref={picker}
            type="file"
            accept="image/png,image/jpeg,image/webp,image/gif"
            hidden
            onChange={(e) => {
              void take(e.target.files?.[0]);
              e.target.value = "";
            }}
          />
          <Button variant="ghost" onClick={() => picker.current?.click()}>
            {value ? "Change" : "Choose…"}
          </Button>
          <Button variant="ghost" onClick={() => setChoosing(true)}>
            From gallery…
          </Button>
          {value && (
            <Button variant="ghost" onClick={() => onPick(null)}>
              Remove
            </Button>
          )}
        </div>
      </div>
      {error && <span className="text-xs text-danger">{error}</span>}
      {help}
      {choosing && (
        <GalleryPicker
          token={token}
          onClose={() => setChoosing(false)}
          onPick={(m) => {
            setChoosing(false);
            setError(null);
            mediaAsInput(token, m).then(onPick, (e: unknown) =>
              setError(e instanceof Error ? e.message : "That picture couldn't be used."),
            );
          }}
        />
      )}
    </div>
  );
}

function sizeLabel(o: string) {
  const [w, h] = o.split("x").map(Number);
  if (!w || !h) return o;
  const shape = w === h ? "Square" : w > h ? "Landscape" : "Portrait";
  return `${shape} ${w}×${h}`;
}

function JobRow({ job: j, packs, onCancel }: { job: Job; packs: Pack[]; onCancel?: () => void }) {
  const name = packs.find((p) => p.id === j.pack_id)?.name ?? (j.pack_id === "train" ? "Training a character" : j.pack_id);
  const prompt = useMemo(() => {
    try {
      return (JSON.parse(j.inputs).prompt as string) ?? "";
    } catch {
      return "";
    }
  }, [j.inputs]);
  const p = j.progress;
  return (
    <li className="flex flex-col gap-2 rounded-xl border border-line bg-surface px-4 py-3">
      <div className="flex items-center gap-3">
        <span className="flex min-w-0 flex-1 flex-col">
          <span className="truncate text-sm">{prompt || name}</span>
          <span className="text-xs text-muted">
            {name} ·{" "}
            {j.status === "queued"
              ? j.position
                ? `${j.position} ahead`
                : "Next up"
              : j.status === "running"
                ? (p?.phase ?? "Running")
                : j.status === "failed"
                  ? "Failed"
                  : j.status}
          </span>
        </span>
        {onCancel && (
          <Button variant="ghost" onClick={onCancel}>
            {j.status === "running" ? "Stop" : "Cancel"}
          </Button>
        )}
      </div>
      {j.status === "running" && (
        <div className="flex flex-col gap-1" aria-live="polite">
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            {p?.percent != null ? (
              <div className="h-2 rounded-full bg-ember transition-all" style={{ width: `${p.percent}%` }} />
            ) : (
              <div className="h-2 w-1/3 animate-pulse rounded-full bg-ember/70" />
            )}
          </div>
          {p && p.steps > 0 && (
            <span className="text-xs text-muted">
              Step {p.step} of {p.steps}
            </span>
          )}
        </div>
      )}
      {j.status === "failed" && j.error && <p className="text-sm text-danger">{j.error}</p>}
    </li>
  );
}

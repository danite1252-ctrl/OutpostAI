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
import { ApiError, createApi, type Job, type Lane, type Media, type Pack, type PackInput } from "../api";
import { dropCached, Lightbox, prepareImage, useAuthedUrl } from "../components/images";
import { Button, ErrorNote } from "../components/ui";

const LANE: Record<Lane, string> = {
  text_to_image: "Text to image",
  edit_image: "Edit an image",
  text_to_video: "Text to video",
  image_to_video: "Image to video",
};

const FIT_NOTE = {
  fits: { text: "Fits your graphics card", style: "text-ok" },
  partial: { text: "Bigger than your graphics card: runs, but slowly", style: "text-warn" },
  too_large: { text: "Too big for your machine", style: "text-danger" },
} as const;

type Values = Record<string, unknown>;

function message(err: unknown) {
  return err instanceof ApiError ? err.message : "Something went wrong.";
}

function defaults(pack: Pack): Values {
  const v: Values = {};
  for (const i of pack.inputs) v[i.key] = i.default ?? (i.kind === "text" ? "" : null);
  return v;
}

export default function Create({
  token,
  canManage,
  comfyInstalled,
  onUnauthorized,
}: {
  token: string;
  canManage: boolean;
  comfyInstalled: boolean;
  onUnauthorized: () => void;
}) {
  const [packs, setPacks] = useState<Pack[]>([]);
  const [packId, setPackId] = useState<string | null>(null);
  const [values, setValues] = useState<Values>({});
  const [loras, setLoras] = useState<{ loras: string[]; folder: string }>({ loras: [], folder: "" });
  const [jobs, setJobs] = useState<Job[]>([]);
  const [gallery, setGallery] = useState<Media[]>([]);
  const [viewing, setViewing] = useState<Media | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const finishedSeen = useRef<Set<string>>(new Set());
  // "Use these settings" from another pack: applied once that pack is showing.
  const pendingReuse = useRef<{ pack: string; values: Values } | null>(null);

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

  const pack = packs.find((p) => p.id === packId) ?? packs[0] ?? null;
  useEffect(() => {
    if (pack && packId !== pack.id) setPackId(pack.id);
  }, [pack, packId]);
  useEffect(() => {
    if (!pack) return;
    const reuse = pendingReuse.current;
    if (reuse && reuse.pack === pack.id) {
      pendingReuse.current = null;
      setValues({ ...defaults(pack), ...reuse.values });
    } else {
      setValues((v) => ({ ...defaults(pack), ...pickKnown(v, pack) }));
    }
  }, [pack?.id]); // eslint-disable-line react-hooks/exhaustive-deps

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
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  }

  async function install(p: Pack) {
    setError(null);
    try {
      await createApi.installPack(token, p.id);
      await loadPacks();
    } catch (err) {
      fail(err);
    }
  }

  async function cancel(j: Job) {
    try {
      await createApi.cancelJob(token, j.id);
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
    const target = m.pack_id && packs.some((p) => p.id === m.pack_id) ? m.pack_id : pack?.id;
    if (!target) return;
    if (target === pack?.id) {
      setValues((v) => ({ ...v, ...restored }));
    } else {
      pendingReuse.current = { pack: target, values: restored };
      setPackId(target);
    }
  }

  const queue = jobs.filter((j) => j.status === "queued" || j.status === "running");
  const recentFailures = jobs.filter((j) => j.status === "failed").slice(0, 3);

  return (
    <section className="mx-auto flex max-w-6xl flex-col gap-6">
      <div>
        <h1 className="text-2xl font-semibold">Create</h1>
        <p className="text-sm text-muted">
          Make images on this PC. Jobs wait their turn for the graphics card, so chat may pause while one runs.
        </p>
      </div>
      {!comfyInstalled && (
        <p className="rounded-2xl border border-warn/40 bg-surface p-4 text-sm" role="status">
          Create runs on ComfyUI. Install it in <b>Settings › Engines</b> first; Outpost starts it when a job needs it.
        </p>
      )}
      <ErrorNote>{error}</ErrorNote>

      <div className="flex flex-wrap gap-2" role="tablist" aria-label="Workflow">
        {packs.map((p) => (
          <button
            key={p.id}
            type="button"
            role="tab"
            aria-selected={p.id === pack?.id}
            onClick={() => setPackId(p.id)}
            className={`flex min-h-11 flex-col items-start rounded-xl border px-4 py-2 text-left ${
              p.id === pack?.id ? "border-ember bg-raised" : "border-line bg-surface hover:bg-raised"
            }`}
          >
            <span className="text-sm font-semibold">{p.name}</span>
            <span className="text-xs text-muted">{LANE[p.lane]}</span>
          </button>
        ))}
      </div>

      {pack && (
        <div className="grid grid-cols-1 gap-6 lg:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]">
          <div className="flex flex-col gap-4 rounded-2xl border border-line bg-surface p-5">
            <div>
              <h2 className="text-lg font-semibold">{pack.name}</h2>
              <p className="text-sm text-soft">{pack.description}</p>
              <p className={`mt-1 text-xs ${FIT_NOTE[pack.fit].style}`}>
                {FIT_NOTE[pack.fit].text} · needs about {pack.vram_gb} GB of video memory
                {pack.license ? ` · ${pack.license}` : ""}
              </p>
            </div>
            {pack.models.ready ? (
              <PackForm
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
        <h2 className="text-lg font-semibold">Gallery</h2>
        {gallery.length === 0 ? (
          <p className="text-sm text-muted">Your pictures will appear here. They're private to your account.</p>
        ) : (
          <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
            {gallery.map((m) => (
              <li key={m.id}>
                <Thumb token={token} media={m} onOpen={() => setViewing(m)} />
              </li>
            ))}
          </ul>
        )}
      </div>

      {viewing && (
        <Viewer
          token={token}
          media={viewing}
          onClose={() => setViewing(null)}
          onReuse={() => void reuse(viewing)}
          onDelete={() => void remove(viewing)}
        />
      )}
    </section>
  );
}

/** Keep values whose keys the pack has (when switching packs, the prompt carries over). */
function pickKnown(v: Values, pack: Pack): Values {
  const out: Values = {};
  for (const i of pack.inputs) if (i.key in v && i.kind === "text") out[i.key] = v[i.key];
  return out;
}

function ModelSetup({ pack, canManage, onInstall }: { pack: Pack; canManage: boolean; onInstall: () => void }) {
  const s = pack.models.setup;
  const running = s?.running ?? false;
  return (
    <div className="flex flex-col gap-3">
      <p className="text-sm">
        This workflow needs <b>{pack.models.missing_gb.toFixed(1)} GB</b> of model files before its first picture. They
        download once and stay in Outpost's models folder.
      </p>
      <ul className="flex flex-col gap-1 text-sm">
        {pack.models.files.map((f) => (
          <li key={f.file} className="flex items-center gap-2">
            <span className={f.present ? "text-ok" : "text-muted"}>{f.present ? "✓" : "○"}</span>
            <span className="font-mono text-xs">{f.file}</span>
            <span className="text-xs text-muted">· {f.size_gb} GB</span>
          </li>
        ))}
      </ul>
      {running && s && (
        <div className="flex flex-col gap-1.5" aria-live="polite">
          <div className="h-2 overflow-hidden rounded-full bg-raised">
            <div className="h-2 rounded-full bg-ember transition-all" style={{ width: `${s.percent ?? 0}%` }} />
          </div>
          <span className="text-xs text-muted">
            {s.current ? `Downloading ${s.current}` : "Starting"} · {gb(s.done_bytes)} of {gb(s.total_bytes)}
          </span>
        </div>
      )}
      {s?.error && !running && <p className="text-sm text-danger">{s.error}</p>}
      {canManage ? (
        !running && (
          <Button onClick={onInstall} disabled={pack.fit === "too_large"}>
            {s?.error ? "Try again" : `Download ${pack.models.missing_gb.toFixed(1)} GB`}
          </Button>
        )
      ) : (
        <p className="text-sm text-muted">Ask the owner to download these models.</p>
      )}
    </div>
  );
}

function gb(bytes: number) {
  return `${(bytes / 1e9).toFixed(1)} GB`;
}

function PackForm({
  pack,
  values,
  loras,
  busy,
  onChange,
  onRefreshLoras,
  onSubmit,
}: {
  pack: Pack;
  values: Values;
  loras: { loras: string[]; folder: string };
  busy: boolean;
  onChange: (key: string, v: unknown) => void;
  onRefreshLoras: () => void;
  onSubmit: (e?: FormEvent) => void;
}) {
  const basic = pack.inputs.filter((i) => !i.advanced);
  const advanced = pack.inputs.filter((i) => i.advanced);
  const missingRequired = pack.inputs.some((i) => i.required && !values[i.key]);
  return (
    <form onSubmit={onSubmit} className="flex flex-col gap-4" noValidate>
      {basic.map((i) => (
        <InputField key={i.key} input={i} value={values[i.key]} loras={loras} onChange={onChange} onRefreshLoras={onRefreshLoras} onSubmit={onSubmit} />
      ))}
      {advanced.length > 0 && (
        <details className="flex flex-col gap-3">
          <summary className="cursor-pointer text-sm text-muted">More settings</summary>
          <div className="mt-3 flex flex-col gap-4">
            {advanced.map((i) => (
              <InputField key={i.key} input={i} value={values[i.key]} loras={loras} onChange={onChange} onRefreshLoras={onRefreshLoras} onSubmit={onSubmit} />
            ))}
          </div>
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
  input: i,
  value,
  loras,
  onChange,
  onRefreshLoras,
  onSubmit,
}: {
  input: PackInput;
  value: unknown;
  loras: { loras: string[]; folder: string };
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
  const help = i.help && <span className="text-xs text-muted">{i.help}</span>;
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
              {loras.loras.map((l) => (
                <option key={l} value={l}>
                  {l.replace(/\.safetensors$/i, "")}
                </option>
              ))}
            </select>
            <Button variant="ghost" onClick={onRefreshLoras} title="Look for new LoRA files">
              Refresh
            </Button>
          </div>
          <span className="text-xs text-muted">
            {i.help} To add one, copy its <span className="font-mono">.safetensors</span> file into{" "}
            <span className="font-mono break-all">{loras.folder || "the LoRA folder"}</span>, then Refresh. Start the
            prompt with its trigger word.
          </span>
        </div>
      );
    case "image":
      return <ImageField id={id} label={label} help={help} value={value as string | null} onPick={(v) => onChange(i.key, v)} />;
  }
}

/** Pick, paste or drop a picture; it is sent with the job (large photos are shrunk). */
function ImageField({
  id,
  label,
  help,
  value,
  onPick,
}: {
  id: string;
  label: ReactNode;
  help: ReactNode;
  value: string | null;
  onPick: (dataUrl: string | null) => void;
}) {
  const [error, setError] = useState<string | null>(null);
  const [over, setOver] = useState(false);
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
          <span className="flex-1 text-sm text-muted">Drop a picture here, paste one, or choose a file.</span>
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
          {value && (
            <Button variant="ghost" onClick={() => onPick(null)}>
              Remove
            </Button>
          )}
        </div>
      </div>
      {error && <span className="text-xs text-danger">{error}</span>}
      {help}
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
  const name = packs.find((p) => p.id === j.pack_id)?.name ?? j.pack_id;
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

function Thumb({ token, media: m, onOpen }: { token: string; media: Media; onOpen: () => void }) {
  const { src, failed } = useAuthedUrl(token, `/api/media/${m.id}`);
  const box = "aspect-square w-full overflow-hidden rounded-xl border border-line bg-raised";
  if (failed) return <div className={`${box} flex items-center justify-center text-xs text-muted`}>Missing</div>;
  if (!src) return <div className={`${box} animate-pulse`} />;
  return (
    <button type="button" onClick={onOpen} className={`${box} block hover:border-ember`} title={m.prompt ?? undefined}>
      {m.kind === "video" ? (
        <video src={src} muted playsInline className="h-full w-full object-cover" />
      ) : (
        <img src={src} alt={m.prompt ?? "Generated image"} className="h-full w-full object-cover" loading="lazy" />
      )}
    </button>
  );
}

function Viewer({
  token,
  media: m,
  onClose,
  onReuse,
  onDelete,
}: {
  token: string;
  media: Media;
  onClose: () => void;
  onReuse: () => void;
  onDelete: () => void;
}) {
  const { src } = useAuthedUrl(token, `/api/media/${m.id}`);
  const [big, setBig] = useState(false);
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div role="dialog" aria-modal="true" aria-label="Picture" className="fixed inset-0 z-40 flex bg-black/80 p-6" onClick={onClose}>
      <div className="mx-auto flex max-h-full w-full max-w-5xl flex-col gap-4 lg:flex-row" onClick={(e) => e.stopPropagation()}>
        <div className="flex min-h-0 flex-1 items-center justify-center">
          {src &&
            (m.kind === "video" ? (
              <video src={src} controls autoPlay loop className="max-h-full max-w-full rounded-xl" />
            ) : (
              <button type="button" onClick={() => setBig(true)} className="max-h-full">
                <img src={src} alt={m.prompt ?? "Generated image"} className="max-h-[80vh] max-w-full rounded-xl object-contain" />
              </button>
            ))}
        </div>
        <aside className="flex w-full flex-col gap-3 rounded-2xl border border-line bg-surface p-4 lg:w-72">
          <p className="text-sm whitespace-pre-wrap">{m.prompt || "No prompt"}</p>
          <p className="font-mono text-xs text-muted">
            {m.pack_id} · seed {m.seed ?? "—"} · {new Date(m.created_at * 1000).toLocaleString()}
          </p>
          <Button onClick={onReuse}>Use these settings</Button>
          <Button variant="danger" onClick={onDelete}>
            Delete
          </Button>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </aside>
        {big && src && <Lightbox src={src} alt={m.prompt ?? "Generated image"} onClose={() => setBig(false)} />}
      </div>
    </div>
  );
}

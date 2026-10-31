import { useEffect, useState } from "react";
import { ApiError, authedBlob, authedFile, createApi, type Media, type Pack, type Task } from "../api";
import { SaveButton } from "./save";
import { Lightbox, prepareImage, useAuthedUrl } from "./images";
import { ModelTile, ModelView } from "./model3d";
import { jobsChanged } from "./queue";
import { Button, ErrorNote, Icon, icons } from "./ui";

/** What the gallery asks Create to do with a picture. */
export type CreateRequest =
  | { kind: "reuse"; media: Media }
  | { kind: "use_image"; media: Media; task: Extract<Task, "edit" | "animate" | "model"> };

/** A gallery picture as a data URL, ready to be an input for another job. */
export async function mediaAsInput(token: string, m: Media): Promise<string> {
  const blob = await authedBlob(token, `/api/media/${m.id}`);
  const ext = m.mime.split("/")[1] ?? "png";
  const file = new File([blob], `gallery.${ext}`, { type: m.mime });
  return (await prepareImage(file)).dataUrl;
}

/** A gallery item in the grid: its picture (or video's first frame), or a cube for a 3D model. */
export function Thumb(props: Parameters<typeof MediaThumb>[0]) {
  const { media: m, onOpen, label, caption } = props;
  if (m.kind !== "model") return <MediaThumb {...props} />;
  // A 3D model has no picture of its own; it isn't downloaded until it's opened.
  const tile = (
    <button
      type="button"
      onClick={onOpen}
      className="relative flex aspect-square w-full flex-col items-center justify-center gap-2 overflow-hidden rounded-xl border border-line bg-raised text-soft hover:border-ember"
      title={m.prompt ?? undefined}
      aria-label={label ?? "Open 3D model"}
    >
      <ModelTile size={44} />
      <span className="text-xs text-muted">3D model</span>
      {m.shared_at && !caption && (
        <span className="absolute top-2 left-2 rounded-md bg-black/70 px-1.5 py-0.5 text-[11px] text-ink">Shared</span>
      )}
    </button>
  );
  if (!caption) return tile;
  return (
    <div className="flex flex-col gap-1">
      {tile}
      <span className="truncate text-xs text-muted">{caption}</span>
    </div>
  );
}

function MediaThumb({
  token,
  media: m,
  onOpen,
  label,
  caption,
}: {
  token: string;
  media: Media;
  onOpen: () => void;
  label?: string;
  /** A line under the picture, e.g. who shared it. */
  caption?: string;
}) {
  const { src, failed } = useAuthedUrl(token, `/api/media/${m.id}`);
  const box = "relative aspect-square w-full overflow-hidden rounded-xl border border-line bg-raised";
  if (failed) return <div className={`${box} flex items-center justify-center text-xs text-muted`}>Missing</div>;
  if (!src) return <div className={`${box} animate-pulse`} />;
  const thumb = (
    <button
      type="button"
      onClick={onOpen}
      className={`${box} block hover:border-ember`}
      title={m.prompt ?? undefined}
      aria-label={label ?? (m.prompt ? `Open: ${m.prompt}` : "Open picture")}
    >
      {m.kind === "video" ? (
        <>
          <video src={src} muted playsInline preload="metadata" className="h-full w-full object-cover" />
          <span className="absolute right-2 bottom-2 rounded-md bg-black/70 px-1.5 py-0.5 text-[11px] text-ink">Video</span>
        </>
      ) : (
        <img src={src} alt={m.prompt ?? "Generated image"} className="h-full w-full object-cover" loading="lazy" />
      )}
      {m.shared_at && !caption && (
        <span className="absolute top-2 left-2 rounded-md bg-black/70 px-1.5 py-0.5 text-[11px] text-ink">Shared</span>
      )}
    </button>
  );
  if (!caption) return thumb;
  return (
    <div className="flex flex-col gap-1">
      {thumb}
      <span className="truncate text-xs text-muted">{caption}</span>
    </div>
  );
}

export function GalleryGrid<M extends Media>({
  token,
  items,
  onOpen,
  caption,
}: {
  token: string;
  items: M[];
  onOpen: (m: M) => void;
  caption?: (m: M) => string;
}) {
  return (
    <ul className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4 xl:grid-cols-5">
      {items.map((m) => (
        <li key={m.id}>
          <Thumb token={token} media={m} onOpen={() => onOpen(m)} caption={caption?.(m)} />
        </li>
      ))}
    </ul>
  );
}

export function Viewer({
  token,
  media: m,
  packName,
  by,
  onClose,
  onRequest,
  onShare,
  onDelete,
}: {
  token: string;
  media: Media;
  packName?: string;
  /** Who made it, when it isn't yours. */
  by?: string;
  onClose: () => void;
  onRequest: (r: CreateRequest) => void;
  /** Put it in the shared gallery or take it out; left out when you can't. */
  onShare?: (shared: boolean) => void;
  /** Left out when you can't delete it. */
  onDelete?: () => void;
}) {
  const { src } = useAuthedUrl(token, `/api/media/${m.id}`);
  const [big, setBig] = useState(false);
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => e.key === "Escape" && !big && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose, big]);
  return (
    <div role="dialog" aria-modal="true" aria-label={m.kind === "video" ? "Video" : m.kind === "model" ? "3D model" : "Picture"} className="fixed inset-0 z-40 flex bg-black/80 p-6" onClick={onClose}>
      <div className="mx-auto flex max-h-full w-full max-w-5xl flex-col gap-4 lg:flex-row" onClick={(e) => e.stopPropagation()}>
        <div className="flex min-h-0 flex-1 items-center justify-center">
          {src &&
            (m.kind === "model" ? (
              <ModelView src={src} className="h-[70vh] w-full" />
            ) : m.kind === "video" ? (
              <video src={src} controls autoPlay loop className="max-h-full max-w-full rounded-xl" />
            ) : (
              <button type="button" onClick={() => setBig(true)} className="max-h-full" aria-label="Show full size">
                <img src={src} alt={m.prompt ?? "Generated image"} className="max-h-[80vh] max-w-full rounded-xl object-contain" />
              </button>
            ))}
        </div>
        <aside className="flex w-full flex-col gap-3 overflow-auto rounded-2xl border border-line bg-surface p-4 lg:w-72">
          <p className="text-sm whitespace-pre-wrap">{m.prompt || (m.kind === "model" ? "3D model" : "No prompt")}</p>
          {m.kind === "model" && <p className="text-xs text-muted">Drag to turn it, scroll to zoom.</p>}
          {m.description && <p className="text-xs text-soft">Outpost sees: {m.description}</p>}
          <p className="font-mono text-xs text-muted">
            {packName ?? m.pack_id} · seed {m.seed ?? "—"} · {new Date(m.created_at * 1000).toLocaleString()}
          </p>
          {(by || m.shared_at) && (
            <p className="text-xs text-soft">
              {by ? `By ${by}` : ""}
              {by && m.shared_at ? " · " : ""}
              {m.shared_at ? "In the shared gallery" : ""}
            </p>
          )}
          {m.kind !== "model" && <Button onClick={() => onRequest({ kind: "reuse", media: m })}>Use these settings</Button>}
          {m.kind === "image" && !by && <PictureTools token={token} media={m} />}
          {m.kind === "image" && (
            <>
              <Button variant="ghost" onClick={() => onRequest({ kind: "use_image", media: m, task: "edit" })}>
                Edit this picture
              </Button>
              <Button variant="ghost" onClick={() => onRequest({ kind: "use_image", media: m, task: "animate" })}>
                Animate this picture
              </Button>
              <Button variant="ghost" onClick={() => onRequest({ kind: "use_image", media: m, task: "model" })}>
                Make a 3D model
              </Button>
            </>
          )}
          {m.kind === "model" && <SaveForPrinting token={token} media={m} />}
          {src && (
            <SaveButton
              key={`copy-${m.id}`}
              label={m.kind === "model" ? "Save for games and 3D apps (.glb)" : "Save a copy"}
              name={`outpost-${m.id.slice(0, 8)}.${extension(m.mime)}`}
              fetchFile={async () => ({ blob: await fetch(src).then((r) => r.blob()) })}
            />
          )}
          {onShare && (
            <Button variant="ghost" onClick={() => onShare(!m.shared_at)}>
              {m.shared_at ? "Stop sharing" : "Share with everyone"}
            </Button>
          )}
          {onDelete && (
            <Button variant="danger" onClick={onDelete}>
              Delete
            </Button>
          )}
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </aside>
        {big && src && <Lightbox src={src} alt={m.prompt ?? "Generated image"} onClose={() => setBig(false)} />}
      </div>
    </div>
  );
}

/** Choose one of my gallery pictures (for an image input). */
export function GalleryPicker({ token, onPick, onClose }: { token: string; onPick: (m: Media) => void; onClose: () => void }) {
  const [items, setItems] = useState<Media[] | null>(null);
  useEffect(() => {
    void createApi.gallery(token).then(
      (list) => setItems(list.filter((m) => m.kind === "image")),
      () => setItems([]),
    );
  }, [token]);
  useEffect(() => {
    const onKey = (e: globalThis.KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div role="dialog" aria-modal="true" aria-label="Choose from your gallery" className="fixed inset-0 z-40 flex bg-black/80 p-6" onClick={onClose}>
      <div className="mx-auto flex max-h-full w-full max-w-4xl flex-col gap-4 rounded-2xl border border-line bg-surface p-5" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-3">
          <h2 className="flex-1 text-lg font-semibold">Choose from your gallery</h2>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="flex size-10 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-ink"
          >
            <Icon d={icons.close} size={18} />
          </button>
        </div>
        <div className="min-h-0 overflow-auto">
          {items === null ? (
            <p className="text-sm text-muted">Loading…</p>
          ) : items.length === 0 ? (
            <p className="text-sm text-muted">No pictures yet.</p>
          ) : (
            <ul className="grid grid-cols-3 gap-3 sm:grid-cols-4 lg:grid-cols-5">
              {items.map((m) => (
                <li key={m.id}>
                  <Thumb token={token} media={m} onOpen={() => onPick(m)} label={m.prompt ? `Use: ${m.prompt}` : "Use this picture"} />
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

const TOOLS = [
  { pack: "upscale_esrgan", label: "Upscale 2×", inputs: { scale: "2×" } },
  { pack: "upscale_esrgan", label: "Upscale 4×", inputs: { scale: "4×" } },
  { pack: "remove_background", label: "Remove background", inputs: {} },
] as const;

/** Upscale or cut out one of my pictures: queued like any job; the result joins the gallery. */
function PictureTools({ token, media: m }: { token: string; media: Media }) {
  const [packs, setPacks] = useState<Pack[] | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = () => createApi.packs(token).then(setPacks, () => setPacks([]));
  useEffect(() => {
    void load();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [token]);
  const downloading = packs?.some((p) => p.lane === "tool" && p.models.setup?.running) ?? false;
  useEffect(() => {
    if (!downloading) return;
    const t = window.setInterval(() => void load(), 2000);
    return () => window.clearInterval(t);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [downloading]);
  if (!packs) return null;
  const tool = (id: string) => packs.find((p) => p.id === id);
  const usable = (p?: Pack) => !!p && (p.models.ready || !!p.runs_on) && p.missing_nodes.length === 0;
  const fail = (e: unknown) => setError(e instanceof ApiError ? e.message : "Something went wrong.");

  const run = (packId: string, label: string, inputs: Record<string, unknown>) => {
    setError(null);
    setNote(null);
    createApi
      .createJob(token, packId, { image: { $media: m.id }, ...inputs })
      .then(() => {
        jobsChanged();
        setNote(`${label}: queued. The result appears in your gallery.`);
      })
      .catch(fail);
  };

  const missing = [...new Set(TOOLS.map((t) => t.pack))].map(tool).filter((p): p is Pack => !!p && !usable(p));
  return (
    <div className="flex flex-col gap-2 rounded-xl border border-line p-3">
      <span className="text-xs font-semibold tracking-wide text-muted uppercase">Picture tools</span>
      <div className="flex flex-wrap gap-2">
        {TOOLS.filter((t) => usable(tool(t.pack))).map((t) => (
          <Button key={t.label} variant="ghost" className="min-h-9 px-3" onClick={() => run(t.pack, t.label, t.inputs)}>
            {t.label}
          </Button>
        ))}
      </div>
      {missing.map((p) => (
        <div key={p.id} className="flex flex-wrap items-center gap-2 text-xs text-muted">
          <span className="flex-1">
            {p.name}:{" "}
            {p.models.setup?.running
              ? `downloading${p.models.setup.percent != null ? ` ${Math.round(p.models.setup.percent)}%` : "…"}`
              : p.missing_nodes.length
                ? "needs a newer ComfyUI"
                : `a ${p.models.missing_gb < 0.1 ? `${Math.round(p.models.missing_gb * 1000)} MB` : `${p.models.missing_gb.toFixed(1)} GB`} download`}
          </span>
          {!p.models.setup?.running && !p.missing_nodes.length && (
            <Button
              variant="ghost"
              className="min-h-8 px-3"
              onClick={() => {
                setError(null);
                createApi.installPack(token, p.id).then(load).catch(fail);
              }}
            >
              Download
            </Button>
          )}
        </div>
      ))}
      {note && <p className="text-xs text-ok" role="status">{note}</p>}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

function extension(mime: string) {
  if (mime === "model/gltf-binary") return "glb";
  if (mime === "image/jpeg") return "jpg";
  return mime.split("/")[1] ?? "png";
}

/** The model as a .3mf for Bambu Studio and other slicers: upright, flat on the bed. */
function SaveForPrinting({ token, media: m }: { token: string; media: Media }) {
  return (
    <>
      <SaveButton
        key={`print-${m.id}`}
        variant="primary"
        label="Save for 3D printing (.3mf)"
        busyLabel="Getting it ready…"
        name={`outpost-${m.id.slice(0, 8)}.3mf`}
        fetchFile={async () => {
          const { blob, headers } = await authedFile(token, `/api/media/${m.id}/print`);
          const filaments = (headers.get("x-outpost-filaments") ?? "").split(",").filter(Boolean);
          return { blob, extra: filaments.length > 1 ? <Filaments colours={filaments} /> : undefined };
        }}
      />
      <p className="text-xs text-muted">Opens in Bambu Studio, PrusaSlicer and Cura, standing on the bed. You can resize it there.</p>
    </>
  );
}

/** The filaments a coloured print uses, in order (load them in the AMS slots 1 to 4). */
function Filaments({ colours }: { colours: string[] }) {
  return (
    <div className="flex flex-col gap-1">
      <span className="text-xs text-muted">Filaments, in Bambu Studio's order (match them to your AMS):</span>
      <ul className="flex flex-wrap gap-2">
        {colours.map((c, i) => (
          <li key={c + i} className="flex items-center gap-1.5 text-xs">
            <span className="inline-block size-4 rounded border border-line" style={{ background: c }} aria-hidden="true" />
            {i + 1}: {c}
          </li>
        ))}
      </ul>
    </div>
  );
}

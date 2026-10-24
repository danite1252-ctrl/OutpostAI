import { useEffect, useState } from "react";
import { authedBlob, createApi, type Media, type Task } from "../api";
import { Lightbox, prepareImage, useAuthedUrl } from "./images";
import { Button, Icon, icons } from "./ui";

/** What the gallery asks Create to do with a picture. */
export type CreateRequest =
  | { kind: "reuse"; media: Media }
  | { kind: "use_image"; media: Media; task: Extract<Task, "edit" | "animate"> };

/** A gallery picture as a data URL, ready to be an input for another job. */
export async function mediaAsInput(token: string, m: Media): Promise<string> {
  const blob = await authedBlob(token, `/api/media/${m.id}`);
  const ext = m.mime.split("/")[1] ?? "png";
  const file = new File([blob], `gallery.${ext}`, { type: m.mime });
  return (await prepareImage(file)).dataUrl;
}

export function Thumb({
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
    <div role="dialog" aria-modal="true" aria-label={m.kind === "video" ? "Video" : "Picture"} className="fixed inset-0 z-40 flex bg-black/80 p-6" onClick={onClose}>
      <div className="mx-auto flex max-h-full w-full max-w-5xl flex-col gap-4 lg:flex-row" onClick={(e) => e.stopPropagation()}>
        <div className="flex min-h-0 flex-1 items-center justify-center">
          {src &&
            (m.kind === "video" ? (
              <video src={src} controls autoPlay loop className="max-h-full max-w-full rounded-xl" />
            ) : (
              <button type="button" onClick={() => setBig(true)} className="max-h-full" aria-label="Show full size">
                <img src={src} alt={m.prompt ?? "Generated image"} className="max-h-[80vh] max-w-full rounded-xl object-contain" />
              </button>
            ))}
        </div>
        <aside className="flex w-full flex-col gap-3 overflow-auto rounded-2xl border border-line bg-surface p-4 lg:w-72">
          <p className="text-sm whitespace-pre-wrap">{m.prompt || "No prompt"}</p>
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
          <Button onClick={() => onRequest({ kind: "reuse", media: m })}>Use these settings</Button>
          {m.kind === "image" && (
            <>
              <Button variant="ghost" onClick={() => onRequest({ kind: "use_image", media: m, task: "edit" })}>
                Edit this picture
              </Button>
              <Button variant="ghost" onClick={() => onRequest({ kind: "use_image", media: m, task: "animate" })}>
                Animate this picture
              </Button>
            </>
          )}
          {src && (
            <a
              href={src}
              download={`outpost-${m.id}.${m.mime.split("/")[1] ?? "png"}`}
              className="flex min-h-11 items-center justify-center rounded-xl border border-line px-4 text-sm hover:bg-raised"
            >
              Save a copy
            </a>
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

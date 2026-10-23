import { useEffect, useState } from "react";
import { fileObjectUrl, MAX_IMAGE_BYTES } from "../api";

/** An image picked for the next message, ready to send. */
export interface PendingImage {
  key: string;
  name: string;
  dataUrl: string;
}

const ACCEPTED = ["image/png", "image/jpeg", "image/webp", "image/gif"];
/** Longest side sent to the model. Vision models scale down anyway; this keeps uploads small. */
const MAX_SIDE = 1600;

/**
 * Read a picked, pasted or dropped file. Large photos are scaled down and sent as
 * JPEG; small images and GIFs go as they are. Throws a message for the user.
 */
export async function prepareImage(file: File): Promise<PendingImage> {
  if (!ACCEPTED.includes(file.type)) throw new Error(`${file.name || "That file"} isn't a PNG, JPEG, WebP or GIF image.`);
  const key = `${Date.now()}-${Math.random().toString(36).slice(2)}`;
  const original = await readAsDataUrl(file);
  if (file.type === "image/gif") {
    if (file.size > MAX_IMAGE_BYTES) throw new Error(`${file.name} is over 10 MB.`);
    return { key, name: file.name, dataUrl: original };
  }
  const img = await loadImage(original);
  const scale = Math.min(1, MAX_SIDE / Math.max(img.naturalWidth, img.naturalHeight));
  if (scale === 1 && file.size <= 3 * 1024 * 1024) return { key, name: file.name, dataUrl: original };

  const canvas = document.createElement("canvas");
  canvas.width = Math.max(1, Math.round(img.naturalWidth * scale));
  canvas.height = Math.max(1, Math.round(img.naturalHeight * scale));
  const ctx = canvas.getContext("2d");
  if (!ctx) return { key, name: file.name, dataUrl: original };
  // JPEG has no transparency; give see-through areas a white background.
  ctx.fillStyle = "#ffffff";
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
  const dataUrl = canvas.toDataURL("image/jpeg", 0.9);
  if (dataUrl.length * 0.75 > MAX_IMAGE_BYTES) throw new Error(`${file.name} is too large even after shrinking.`);
  return { key, name: file.name, dataUrl };
}

function readAsDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(new Error(`${file.name} couldn't be read.`));
    reader.readAsDataURL(file);
  });
}

function loadImage(src: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error("That image couldn't be opened."));
    img.src = src;
  });
}

/**
 * Object URLs for attachments already fetched, so scrolling a chat doesn't refetch
 * them. Kept small: the oldest are released (revoked) once there are too many.
 */
const cache = new Map<string, Promise<string>>();
const CACHE_LIMIT = 48;

function remember(id: string, p: Promise<string>) {
  cache.set(id, p);
  while (cache.size > CACHE_LIMIT) {
    const [oldest, url] = cache.entries().next().value as [string, Promise<string>];
    cache.delete(oldest);
    void url.then((u) => URL.revokeObjectURL(u), () => undefined);
  }
}

/** Release every cached image (on sign-out). */
export function clearImageCache() {
  for (const p of cache.values()) void p.then((u) => URL.revokeObjectURL(u), () => undefined);
  cache.clear();
}

/** An attached image from the gateway (fetched with the token). Click to view it larger. */
export function AttachmentImage({ token, id, alt }: { token: string; id: string; alt: string }) {
  const [src, setSrc] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);
  const [open, setOpen] = useState(false);

  useEffect(() => {
    let live = true;
    let p = cache.get(id);
    if (p) {
      // Most recently used goes to the back of the queue.
      cache.delete(id);
      cache.set(id, p);
    } else {
      p = fileObjectUrl(token, id);
      remember(id, p);
      p.catch(() => cache.delete(id));
    }
    p.then(
      (url) => live && setSrc(url),
      () => live && setFailed(true),
    );
    return () => {
      live = false;
    };
  }, [token, id]);

  if (failed) {
    return <span className="flex h-24 w-32 items-center justify-center rounded-xl bg-raised text-xs text-muted">Image missing</span>;
  }
  if (!src) return <span className="block h-24 w-32 animate-pulse rounded-xl bg-raised" aria-label="Loading image" />;
  return (
    <>
      <button type="button" onClick={() => setOpen(true)} className="overflow-hidden rounded-xl border border-line">
        <img src={src} alt={alt} className="max-h-56 max-w-full object-contain" />
      </button>
      {open && <Lightbox src={src} alt={alt} onClose={() => setOpen(false)} />}
    </>
  );
}

export function Lightbox({ src, alt, onClose }: { src: string; alt: string; onClose: () => void }) {
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);
  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={alt}
      onClick={onClose}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/80 p-8"
    >
      <img src={src} alt={alt} className="max-h-full max-w-full rounded-xl object-contain" />
    </div>
  );
}

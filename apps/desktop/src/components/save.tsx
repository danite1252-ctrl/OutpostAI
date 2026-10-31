// Saving a file from Outpost onto this PC: it goes into the Downloads folder (a new name if
// that one's taken), and the app says where, with a button to show it in its folder.

import { useEffect, useState, type ReactNode } from "react";
import { Button, ErrorNote } from "./ui";

/** Save `blob` into Downloads as `name`; resolves with the full path it was saved to. */
export async function saveToDownloads(blob: Blob, name: string): Promise<string> {
  const bytes = new Uint8Array(await blob.arrayBuffer());
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<string>("save_download", bytes, { headers: { "x-file-name": encodeURIComponent(name) } });
  } catch (e) {
    // Outside the desktop app (a browser while developing): the browser's own download.
    if (!(window as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__) {
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = name;
      a.click();
      window.setTimeout(() => URL.revokeObjectURL(url), 10_000);
      return name;
    }
    throw new Error(typeof e === "string" ? e : "Couldn't save it.");
  }
}

export async function showInFolder(path: string) {
  const { invoke } = await import("@tauri-apps/api/core");
  await invoke("show_in_folder", { path });
}

/** Where a file went, with "Show in folder". */
export function SavedNote({ path, extra }: { path: string; extra?: ReactNode }) {
  const [error, setError] = useState<string | null>(null);
  const file = path.split(/[\\/]/).pop() ?? path;
  return (
    <div className="flex flex-col gap-1.5 rounded-xl border border-line bg-raised px-3 py-2" role="status" aria-live="polite">
      <span className="text-sm">
        Saved to Downloads as <b className="break-all">{file}</b>
      </span>
      {extra}
      <button
        type="button"
        className="self-start text-xs text-muted underline hover:text-ink"
        onClick={() => void showInFolder(path).catch((e: unknown) => setError(e instanceof Error ? e.message : String(e)))}
      >
        Show in folder
      </button>
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  );
}

/**
 * A button that fetches a file and saves it into Downloads, then says where it went.
 * `fetchFile` returns the file (and optionally something to show with the note).
 */
export function SaveButton({
  label,
  name,
  fetchFile,
  variant = "ghost",
  busyLabel = "Saving…",
}: {
  label: string;
  name: string;
  fetchFile: () => Promise<{ blob: Blob; extra?: ReactNode }>;
  variant?: "primary" | "ghost";
  busyLabel?: string;
}) {
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<{ path: string; extra?: ReactNode } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const { blob, extra } = await fetchFile();
      setSaved({ path: await saveToDownloads(blob, name), extra });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't save it.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <Button variant={variant} onClick={() => void save()} disabled={busy}>
        {busy ? busyLabel : label}
      </Button>
      {saved && <SavedNote path={saved.path} extra={saved.extra} />}
      <ErrorNote>{error}</ErrorNote>
    </>
  );
}

/** Files saved from elsewhere in the app (documents in chat): a note in the corner for a while. */
export function SavedToasts() {
  const [path, setPath] = useState<string | null>(null);
  useEffect(() => {
    let t = 0;
    const on = (e: Event) => {
      setPath((e as CustomEvent<string>).detail);
      window.clearTimeout(t);
      t = window.setTimeout(() => setPath(null), 10_000);
    };
    window.addEventListener("outpost-saved", on);
    return () => {
      window.removeEventListener("outpost-saved", on);
      window.clearTimeout(t);
    };
  }, []);
  if (!path) return null;
  return (
    <div className="fixed right-4 bottom-4 z-50 w-80 shadow-lg">
      <SavedNote path={path} />
    </div>
  );
}

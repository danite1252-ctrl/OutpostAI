// Chatting with documents: the documents a chat (or persona) uses, and the library to pick
// them from. Documents are read on the server; while one is being read it says so.

import { saveToDownloads } from "./save";
import { useCallback, useEffect, useRef, useState } from "react";
import { ApiError, authedBlob, DOC_TYPES, docsApi, type Doc, type Source } from "../api";
import { Button, ErrorNote, Icon, icons } from "./ui";

function message(err: unknown) {
  return err instanceof ApiError ? err.message : "Something went wrong.";
}

const size = (b: number) => (b >= 1024 * 1024 ? `${(b / 1024 / 1024).toFixed(1)} MB` : `${Math.max(1, Math.round(b / 1024))} KB`);

function describe(d: Doc) {
  if (d.status === "reading") return "Reading…";
  if (d.status === "failed") return d.error ?? "Couldn't be read";
  return [d.pages ? `${d.pages} page${d.pages === 1 ? "" : "s"}` : null, size(d.bytes)].filter(Boolean).join(" · ");
}

/** Save one of my documents' files. */
async function download(token: string, d: { id: string; name: string }) {
  const blob = await authedBlob(token, `/api/documents/${d.id}/file`);
  const path = await saveToDownloads(blob, d.name);
  // Say where it went, and offer the folder.
  window.dispatchEvent(new CustomEvent("outpost-saved", { detail: path }));
}

/**
 * Pick documents (for a chat or a persona) from my library, upload new ones, or delete them.
 * `chosen` are the ones in use now; `onSave` gets the new list.
 */
export function DocumentPicker({
  token,
  title,
  note,
  chosen,
  onSave,
  onClose,
}: {
  token: string;
  title: string;
  note?: string;
  chosen: string[];
  onSave: (ids: string[]) => Promise<void>;
  onClose: () => void;
}) {
  const [docs, setDocs] = useState<Doc[] | null>(null);
  const [picked, setPicked] = useState<string[]>(chosen);
  const [uploading, setUploading] = useState<{ name: string; fraction: number } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const input = useRef<HTMLInputElement>(null);

  const load = useCallback(() => docsApi.list(token).then(setDocs, (e) => setError(message(e))), [token]);
  useEffect(() => void load(), [load]);
  const reading = docs?.some((d) => d.status === "reading") ?? false;
  useEffect(() => {
    if (!reading) return;
    const t = window.setInterval(() => void load(), 1500);
    return () => window.clearInterval(t);
  }, [reading, load]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  async function upload(files: File[]) {
    setError(null);
    for (const f of files) {
      setUploading({ name: f.name, fraction: 0 });
      try {
        const { document } = await docsApi.add(token, f, (fraction) => setUploading({ name: f.name, fraction }));
        // New documents are ticked for this chat straight away.
        setPicked((p) => (p.includes(document.id) ? p : [...p, document.id]));
      } catch (e) {
        setError(`${f.name}: ${message(e)}`);
      }
    }
    setUploading(null);
    void load();
  }

  async function remove(d: Doc) {
    if (!window.confirm(`Delete ${d.name}? It leaves every chat and persona that uses it.`)) return;
    try {
      await docsApi.remove(token, d.id);
      setPicked((p) => p.filter((x) => x !== d.id));
      void load();
    } catch (e) {
      setError(message(e));
    }
  }

  return (
    <div role="dialog" aria-modal="true" aria-label={title} className="fixed inset-0 z-40 flex items-center justify-center bg-black/80 p-6" onClick={onClose}>
      <div className="flex max-h-full w-full max-w-xl flex-col gap-4 rounded-2xl border border-line bg-surface p-6" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-3">
          <h2 className="flex-1 text-lg font-semibold">{title}</h2>
          <button type="button" onClick={onClose} aria-label="Close" className="rounded-lg p-1 text-muted hover:text-ink">
            <Icon d={icons.close} />
          </button>
        </div>
        <p className="text-sm text-muted">
          {note ??
            "Tick the documents to ask about. Each message gets the passages that fit it, and replies say where they found things."}{" "}
          PDF, Word (.docx), text, Markdown, CSV or HTML, up to 50 MB.
        </p>
        <input
          ref={input}
          type="file"
          accept={DOC_TYPES}
          multiple
          hidden
          onChange={(e) => {
            void upload(Array.from(e.target.files ?? []));
            e.target.value = "";
          }}
        />
        <div className="flex flex-wrap items-center gap-3">
          <Button variant="ghost" disabled={!!uploading} onClick={() => input.current?.click()}>
            Add documents
          </Button>
          {uploading && (
            <span className="text-sm text-muted" aria-live="polite">
              Uploading {uploading.name}… {Math.round(uploading.fraction * 100)}%
            </span>
          )}
        </div>
        <ul className="flex min-h-0 flex-col divide-y divide-line overflow-auto rounded-xl border border-line">
          {docs === null && <li className="px-4 py-3 text-sm text-muted">Loading…</li>}
          {docs?.length === 0 && <li className="px-4 py-3 text-sm text-muted">No documents yet. Add some above.</li>}
          {docs?.map((d) => (
            <li key={d.id} className="flex items-center gap-3 px-4 py-2.5">
              <input
                type="checkbox"
                className="size-4 accent-ember"
                aria-label={`Use ${d.name}`}
                checked={picked.includes(d.id)}
                disabled={d.status === "failed"}
                onChange={(e) => setPicked((p) => (e.target.checked ? [...p, d.id] : p.filter((x) => x !== d.id)))}
              />
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-sm font-medium" title={d.name}>
                  {d.name}
                </span>
                <span className={`text-xs ${d.status === "failed" ? "text-danger" : "text-muted"}`}>{describe(d)}</span>
              </span>
              <button
                type="button"
                className="text-xs text-muted underline hover:text-ink"
                onClick={() => void download(token, d).catch((e) => setError(message(e)))}
              >
                Save
              </button>
              <button type="button" className="text-xs text-muted underline hover:text-danger" onClick={() => void remove(d)}>
                Delete
              </button>
            </li>
          ))}
        </ul>
        <ErrorNote>{error}</ErrorNote>
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            disabled={saving || !!uploading}
            onClick={() => {
              setSaving(true);
              onSave(picked)
                .then(onClose)
                .catch((e) => setError(message(e)))
                .finally(() => setSaving(false));
            }}
          >
            {saving ? "Saving…" : "Use these"}
          </Button>
        </div>
      </div>
    </div>
  );
}

/** The documents above a chat: its own (removable) and its persona's. */
export function ChatDocuments({
  token,
  chatId,
  ensureChat,
  open,
  onOpenChange,
}: {
  token: string;
  /** The chat, once there is one. */
  chatId: string | null;
  /** Make the chat if there isn't one yet (attaching to a new chat). */
  ensureChat: () => Promise<string>;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [mine, setMine] = useState<Doc[]>([]);
  const [persona, setPersona] = useState<Doc[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(() => {
    if (!chatId) {
      setMine([]);
      setPersona([]);
      return;
    }
    docsApi.forChat(token, chatId).then(
      (r) => {
        setMine(r.documents);
        setPersona(r.persona_documents);
      },
      () => undefined,
    );
  }, [token, chatId]);
  useEffect(load, [load]);
  // While something is being read, check back.
  const reading = [...mine, ...persona].some((d) => d.status === "reading");
  useEffect(() => {
    if (!reading) return;
    const t = window.setInterval(load, 2000);
    return () => window.clearInterval(t);
  }, [reading, load]);

  const save = async (ids: string[]) => {
    const id = chatId ?? (await ensureChat());
    const r = await docsApi.setForChat(token, id, ids);
    setMine(r.documents);
    setPersona(r.persona_documents);
  };
  const all = [...mine.map((d) => ({ d, own: true })), ...persona.filter((p) => !mine.some((m) => m.id === p.id)).map((d) => ({ d, own: false }))];

  return (
    <>
      {all.length > 0 && (
        <div className="flex flex-wrap items-center gap-2 border-b border-line px-6 py-2">
          <span className="text-xs text-muted">Using</span>
          {all.map(({ d, own }) => (
            <span
              key={d.id}
              className={`flex items-center gap-1.5 rounded-full border px-2.5 py-0.5 text-xs ${d.status === "failed" ? "border-danger text-danger" : "border-line"}`}
              title={own ? describe(d) : `From the persona · ${describe(d)}`}
            >
              {d.name}
              {d.status === "reading" && <span className="text-muted">(reading…)</span>}
              {!own && <span className="text-muted">(persona)</span>}
              {own && (
                <button
                  type="button"
                  aria-label={`Stop using ${d.name}`}
                  className="text-muted hover:text-ink"
                  onClick={() => void save(mine.filter((m) => m.id !== d.id).map((m) => m.id)).catch((e) => setError(message(e)))}
                >
                  ×
                </button>
              )}
            </span>
          ))}
          <ErrorNote>{error}</ErrorNote>
        </div>
      )}
      {open && (
        <DocumentPicker
          token={token}
          title="Documents for this chat"
          chosen={mine.map((d) => d.id)}
          onSave={save}
          onClose={() => onOpenChange(false)}
        />
      )}
    </>
  );
}

/** Under a reply: where it found things ("[2] Handbook.pdf, page 3"). */
export function SourcesLine({ token, sources }: { token: string; sources: Source[] }) {
  if (sources.length === 0) return null;
  return (
    <div className="mt-2 flex flex-wrap items-center gap-1.5 text-xs text-muted">
      <span>Sources:</span>
      {sources.map((s) => (
        <button
          key={s.n}
          type="button"
          title="Save the document"
          onClick={() => void download(token, { id: s.document_id, name: s.name }).catch(() => undefined)}
          className="rounded-full border border-line px-2 py-0.5 hover:bg-raised hover:text-ink"
        >
          [{s.n}] {s.name}
          {s.page != null ? `, page ${s.page}` : ""}
        </button>
      ))}
    </div>
  );
}

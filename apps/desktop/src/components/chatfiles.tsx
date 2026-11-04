// Files in chats (desktop): the folders a chat works with (on this PC or on the server),
// whether it asks before changing files, what each reply did with files, and the changes
// to apply, skip or undo (with what changed, line by line).

import { useCallback, useEffect, useRef, useState } from "react";
import {
  ApiError,
  authedBlob,
  filesApi,
  pcFolders,
  type ChatFolder,
  type FileAction,
  type FileChange,
  type Folder,
  type PcFolder,
} from "../api";
import { saveToDownloads } from "./save";
import { Button, ErrorNote, ToggleRow, areYouSure } from "./ui";

const SKIP = new Set([".git", "node_modules", "target", "dist", "build", ".next", ".venv", "venv", "__pycache__", ".idea", ".vs"]);

function message(e: unknown) {
  if (e instanceof ApiError) return e.message;
  if (typeof e === "string") return e;
  return e instanceof Error ? e.message : "Something went wrong.";
}

/** A chat's folders, its changes, and this PC's folders. */
export function useChatFiles(token: string, chatId: string | null) {
  const [folders, setFolders] = useState<ChatFolder[]>([]);
  const [changes, setChanges] = useState<FileChange[]>([]);
  const [local, setLocal] = useState<PcFolder[]>([]);
  const reload = useCallback(async () => {
    setLocal(await pcFolders.list().catch(() => []));
    if (!chatId) {
      setFolders([]);
      setChanges([]);
      return;
    }
    const [f, c] = await Promise.all([
      filesApi.chatFolders(token, chatId).catch(() => [] as ChatFolder[]),
      filesApi.changes(token, chatId).catch(() => [] as FileChange[]),
    ]);
    setFolders(f);
    setChanges(c);
  }, [token, chatId]);
  useEffect(() => {
    void reload();
  }, [reload]);
  // The chat's PC folders this app can reach.
  const reachable = folders.filter((f) => f.kind === "pc" && local.some((l) => l.id === f.id)).map((f) => f.id);
  return { folders, setFolders, changes, setChanges, local, setLocal, reachable, reload };
}

export type ChatFiles = ReturnType<typeof useChatFiles>;

/** The header button: how many folders, and the panel. */
export function FilesButton({
  token,
  chatId,
  mode,
  files,
  onMode,
  ensureChat,
}: {
  token: string;
  chatId: string | null;
  mode: "ask" | "auto";
  files: ChatFiles;
  onMode: (m: "ask" | "auto") => void;
  ensureChat: () => Promise<string>;
}) {
  const [open, setOpen] = useState(false);
  const n = files.folders.length;
  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        title="Folders this chat can read and change"
        className="flex min-h-9 items-center rounded-lg border border-line px-3 text-sm hover:bg-raised"
      >
        Files{n ? ` · ${n}` : ""}
      </button>
      {open && (
        <FilesPanel token={token} chatId={chatId} mode={mode} files={files} onMode={onMode} ensureChat={ensureChat} onClose={() => setOpen(false)} />
      )}
    </>
  );
}

export function FilesPanel({
  token,
  chatId,
  mode,
  files,
  onMode,
  ensureChat,
  onClose,
}: {
  token: string;
  chatId: string | null;
  mode: "ask" | "auto";
  files: ChatFiles;
  onMode: (m: "ask" | "auto") => void;
  ensureChat: () => Promise<string>;
  onClose: () => void;
}) {
  const [server, setServer] = useState<Folder[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [newName, setNewName] = useState("");
  const pick = useRef<HTMLInputElement>(null);
  const pickFiles = useRef<HTMLInputElement>(null);
  const [uploadTo, setUploadTo] = useState<Folder | null>(null);

  useEffect(() => {
    filesApi.folders(token).then(setServer, () => undefined);
  }, [token]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const save = async (list: ChatFolder[]) => {
    setError(null);
    try {
      const id = chatId ?? (await ensureChat());
      files.setFolders(await filesApi.setChatFolders(token, id, list));
    } catch (e) {
      setError(message(e));
    }
  };
  const pcName = "this PC";

  async function addPc() {
    setError(null);
    try {
      const f = await pcFolders.pick();
      if (!f) return;
      files.setLocal(await pcFolders.list());
      if (files.folders.some((x) => x.kind === "pc" && x.id === f.id)) return;
      const device = await pcLabel();
      await save([...files.folders, { kind: "pc", id: f.id, name: f.name, device }]);
    } catch (e) {
      setError(message(e));
    }
  }

  async function upload(folder: Folder, list: FileList | null) {
    if (!list || list.length === 0) return;
    const all = Array.from(list).filter((f) => {
      const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
      return !rel.split("/").some((part) => SKIP.has(part)) && f.size <= 50 * 1024 * 1024;
    });
    let done = 0;
    setError(null);
    for (const f of all) {
      // A folder's files arrive as "<folder>/<path>": keep the path inside it.
      const rel = (f as File & { webkitRelativePath?: string }).webkitRelativePath || f.name;
      const path = rel.includes("/") ? rel.split("/").slice(1).join("/") : rel;
      setBusy(`Uploading ${done + 1} of ${all.length}: ${path}`);
      try {
        await filesApi.putFile(token, folder.id, path, f);
      } catch (e) {
        setError(message(e));
      }
      done += 1;
    }
    setBusy(null);
  }

  async function newServerFolder(list: FileList | null, name?: string) {
    setError(null);
    try {
      const first = list?.[0] as (File & { webkitRelativePath?: string }) | undefined;
      const guess = name?.trim() || first?.webkitRelativePath?.split("/")[0] || "Folder";
      const folder = await filesApi.createFolder(token, guess);
      setServer((s) => [...s, folder].sort((a, b) => a.name.localeCompare(b.name)));
      await upload(folder, list);
      await save([...files.folders, { kind: "server", id: folder.id, name: folder.name }]);
      setNewName("");
    } catch (e) {
      setError(message(e));
    }
  }

  const where = (f: ChatFolder) =>
    f.kind === "server"
      ? "On the server"
      : files.local.some((l) => l.id === f.id)
        ? `On ${pcName}: ${files.local.find((l) => l.id === f.id)?.path}`
        : `On ${f.device ?? "another PC"} (usable from the desktop app there)`;

  return (
    <div role="dialog" aria-modal="true" aria-label="Files for this chat" className="fixed inset-0 z-40 flex justify-end bg-black/60" onClick={onClose}>
      <div className="flex h-full w-full max-w-md flex-col gap-4 overflow-auto border-l border-line bg-surface p-5" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center gap-3">
          <h2 className="flex-1 text-lg font-semibold">Files for this chat</h2>
          <Button variant="ghost" onClick={onClose}>
            Close
          </Button>
        </div>
        <p className="text-sm text-muted">
          The chat's model can read, search and change files in these folders (it needs a model with the tools badge).
        </p>
        <ToggleRow
          title="Ask before changing files"
          hint={mode === "ask" ? "Each change waits for you to Apply or Skip it." : "Changes are saved straight away; each has Undo."}
          checked={mode === "ask"}
          onChange={(on) => onMode(on ? "ask" : "auto")}
        />

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Folders</h3>
          {files.folders.length === 0 && <p className="text-sm text-muted">None yet.</p>}
          <ul className="flex flex-col gap-2">
            {files.folders.map((f) => {
              const s = server.find((x) => x.id === f.id);
              return (
                <li key={`${f.kind}-${f.id}`} className="flex flex-col gap-1 rounded-xl border border-line px-3 py-2">
                  <div className="flex items-center gap-2">
                    <span className="flex-1 font-medium">{f.name}</span>
                    <button
                      type="button"
                      className="text-xs text-muted underline hover:text-ink"
                      onClick={() => void save(files.folders.filter((x) => !(x.kind === f.kind && x.id === f.id)))}
                    >
                      Remove from chat
                    </button>
                  </div>
                  <span className="truncate text-xs text-muted">{where(f)}</span>
                  {f.kind === "server" && s && (
                    <div className="flex flex-wrap gap-3 text-xs">
                      <button
                        type="button"
                        className="text-muted underline hover:text-ink"
                        onClick={() => {
                          setUploadTo(s);
                          pickFiles.current?.click();
                        }}
                      >
                        Add files
                      </button>
                      <button
                        type="button"
                        className="text-muted underline hover:text-ink"
                        onClick={() =>
                          void authedBlob(token, `/api/folders/${s.id}/zip`)
                            .then((b) => saveToDownloads(b, `${s.name}.zip`))
                            .then((p) => window.dispatchEvent(new CustomEvent("outpost-saved", { detail: p })))
                            .catch((e) => setError(message(e)))
                        }
                      >
                        Download as zip
                      </button>
                    </div>
                  )}
                </li>
              );
            })}
          </ul>
        </section>

        <section className="flex flex-col gap-2">
          <h3 className="text-sm font-semibold">Add a folder</h3>
          <Button variant="ghost" onClick={() => void addPc()}>
            A folder on {pcName}…
          </Button>
          {server.filter((s) => !files.folders.some((f) => f.kind === "server" && f.id === s.id)).length > 0 && (
            <label className="flex flex-col gap-1 text-sm text-muted">
              A server folder
              <select
                value=""
                onChange={(e) => {
                  const s = server.find((x) => x.id === e.target.value);
                  if (s) void save([...files.folders, { kind: "server", id: s.id, name: s.name }]);
                }}
                className="min-h-10 rounded-lg border border-line bg-deep px-2 text-ink"
              >
                <option value="">Choose…</option>
                {server
                  .filter((s) => !files.folders.some((f) => f.kind === "server" && f.id === s.id))
                  .map((s) => (
                    <option key={s.id} value={s.id}>
                      {s.name}
                    </option>
                  ))}
              </select>
            </label>
          )}
          <div className="flex flex-col gap-2 rounded-xl border border-line p-3">
            <span className="text-sm">A new server folder</span>
            <span className="text-xs text-muted">
              Upload a folder from this PC to the server (for chats from any device, the phone too). Changes happen on the
              server's copy; download it as a zip when you're done.
            </span>
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              placeholder="Name (or the uploaded folder's)"
              maxLength={60}
              className="min-h-10 rounded-lg border border-line bg-deep px-3 text-ink"
            />
            <div className="flex flex-wrap gap-2">
              <Button variant="ghost" onClick={() => pick.current?.click()}>
                Upload a folder…
              </Button>
              <Button variant="ghost" disabled={!newName.trim()} onClick={() => void newServerFolder(null, newName)}>
                Make it empty
              </Button>
            </div>
          </div>
          {busy && (
            <p className="text-xs text-muted" aria-live="polite">
              {busy}
            </p>
          )}
        </section>
        <ErrorNote>{error}</ErrorNote>
        <input
          ref={pick}
          type="file"
          className="hidden"
          // A whole folder (with its sub-folders).
          {...({ webkitdirectory: "", directory: "" } as Record<string, string>)}
          multiple
          onChange={(e) => {
            void newServerFolder(e.target.files, newName);
            e.target.value = "";
          }}
        />
        <input
          ref={pickFiles}
          type="file"
          className="hidden"
          multiple
          onChange={(e) => {
            if (uploadTo) void upload(uploadTo, e.target.files);
            e.target.value = "";
          }}
        />
      </div>
    </div>
  );
}

async function pcLabel(): Promise<string> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<string>("pc_name");
  } catch {
    return "a PC";
  }
}

// ---------------------------------------------------------------- what replies did

const VERB: Record<FileAction["op"], string> = {
  list: "Looked in",
  read: "Read",
  search: "Searched",
  write: "Wrote",
  edit: "Changed",
  tool: "Tried",
  mcp: "Used",
  run: "Ran in",
};

/** Under a reply: what it did with files, and its changes to apply, skip or undo. */
export function FileActions({
  token,
  chatId,
  actions,
  files,
  live,
}: {
  token: string;
  chatId: string | null;
  actions: FileAction[];
  files: ChatFiles;
  live?: boolean;
}) {
  const looked = actions.filter((a) => !a.change_id && a.op !== "run");
  const runs = actions.filter((a) => a.op === "run");
  const changed = actions.filter((a) => a.change_id);
  return (
    <div className="mt-2 flex flex-col gap-2 rounded-xl border border-line bg-surface px-3 py-2 text-sm">
      {runs.map((a, i) => (
        <p key={`run-${i}`} className={`font-mono text-xs break-all ${a.failed ? "text-danger" : "text-ok"}`}>
          Ran in {a.path}
          {a.note ? ` · ${a.note}` : ""}
        </p>
      ))}
      {looked.length > 0 && (
        <details open={live}>
          <summary className="cursor-pointer text-muted">
            {live
              ? looked.some((a) => a.op === "mcp")
                ? "Using tools…"
                : "Working with files…"
              : looked.every((a) => a.op === "mcp")
                ? `Used tools (${looked.length})`
                : looked.some((a) => a.op === "mcp")
                  ? `Used tools and files (${looked.length})`
                  : `Looked at files (${looked.length})`}
          </summary>
          <ul className="mt-1 flex flex-col gap-0.5">
            {looked.map((a, i) => (
              <li key={i} className={`truncate font-mono text-xs ${a.failed ? "text-danger" : "text-muted"}`}>
                {VERB[a.op] ?? a.op} {a.path}
                {a.note ? ` · ${a.note}` : ""}
              </li>
            ))}
          </ul>
        </details>
      )}
      {changed.map((a) => (
        <ChangeRow key={a.change_id!} token={token} chatId={chatId} action={a} files={files} />
      ))}
    </div>
  );
}

function ChangeRow({ token, chatId, action, files }: { token: string; chatId: string | null; action: FileAction; files: ChatFiles }) {
  const change = files.changes.find((c) => c.id === action.change_id);
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const status = change?.status;
  const onPc = change?.kind === "pc";
  const reachable = !onPc || files.local.some((l) => l.id === change?.folder_id);

  async function act(what: "apply" | "skip" | "undo", force = false) {
    if (!change || !chatId) return;
    setBusy(true);
    setError(null);
    try {
      if (onPc && what !== "skip") {
        // This PC makes the change; Outpost records it.
        const expect = what === "apply" ? change.before : change.after;
        const becomes = what === "apply" ? change.after : change.before;
        const now = await pcFolders.op(change.folder_id, "read", change.path);
        if (now.error) throw new Error(String(now.error));
        const current = now.missing ? null : String(now.text ?? "");
        if (current !== expect && !force) {
          if (!await areYouSure(`${change.path} has changed since. ${what === "apply" ? "Apply" : "Undo"} anyway?`)) return;
        }
        const done = becomes === null ? await pcFolders.op(change.folder_id, "delete", change.path) : await pcFolders.op(change.folder_id, "write", change.path, { content: becomes });
        if (done.error) throw new Error(String(done.error));
        const updated = await filesApi.act(token, chatId, change.id, what, { done_on_pc: true });
        files.setChanges((cs) => cs.map((c) => (c.id === updated.id ? updated : c)));
        return;
      }
      const updated = await filesApi.act(token, chatId, change.id, what, { force });
      files.setChanges((cs) => cs.map((c) => (c.id === updated.id ? updated : c)));
    } catch (e) {
      if (e instanceof ApiError && e.code === "changed_since") {
        if (await areYouSure(`${e.message}`)) {
          setBusy(false);
          return act(what, true);
        }
        return;
      }
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  const label =
    status === "proposed"
      ? "Waiting for you"
      : status === "applied"
        ? "Applied"
        : status === "skipped"
          ? "Skipped"
          : status === "undone"
            ? "Undone"
            : "";
  return (
    <div className="flex flex-col gap-1.5">
      <div className="flex flex-wrap items-center gap-2">
        <span className={`font-mono text-xs ${action.failed ? "text-danger" : ""}`}>
          {VERB[action.op]} {action.path}
        </span>
        {action.note && <span className="text-xs text-muted">{action.note}</span>}
        {label && (
          <span className={`text-xs ${status === "proposed" ? "text-amber" : status === "applied" ? "text-ok" : "text-muted"}`}>
            · {label}
          </span>
        )}
        <span className="ml-auto flex gap-2">
          {change && (
            <button type="button" className="text-xs text-muted underline hover:text-ink" onClick={() => setShow((s) => !s)}>
              {show ? "Hide change" : "Show change"}
            </button>
          )}
          {status === "proposed" && reachable && (
            <>
              <Button disabled={busy} onClick={() => void act("apply")}>
                Apply
              </Button>
              <Button variant="ghost" disabled={busy} onClick={() => void act("skip")}>
                Skip
              </Button>
            </>
          )}
          {status === "applied" && reachable && (
            <Button variant="ghost" disabled={busy} onClick={() => void act("undo")}>
              Undo
            </Button>
          )}
        </span>
      </div>
      {!reachable && (status === "proposed" || status === "applied") && (
        <span className="text-xs text-muted">This file is on another PC: open the chat in the desktop app there to {status === "proposed" ? "apply it" : "undo it"}.</span>
      )}
      {show && change && <DiffView before={change.before} after={change.after} />}
      {error && <span className="text-xs text-danger">{error}</span>}
    </div>
  );
}

// ---------------------------------------------------------------- what changed

type Line = { kind: " " | "+" | "-"; text: string };

/** Lines added and removed, in order (longest common run; plain before/after for huge files). */
export function diffLines(before: string | null, after: string): Line[] | null {
  const a = (before ?? "").split("\n");
  const b = after.split("\n");
  if (before === null) return b.map((text) => ({ kind: "+", text }));
  // Trim the shared start and end, then compare what's left.
  let start = 0;
  while (start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const x = a.slice(start, endA);
  const y = b.slice(start, endB);
  if (x.length * y.length > 4_000_000) return null;
  const m = x.length;
  const n = y.length;
  const dp: Uint32Array[] = Array.from({ length: m + 1 }, () => new Uint32Array(n + 1));
  for (let i = m - 1; i >= 0; i--) for (let j = n - 1; j >= 0; j--) dp[i][j] = x[i] === y[j] ? dp[i + 1][j + 1] + 1 : Math.max(dp[i + 1][j], dp[i][j + 1]);
  const mid: Line[] = [];
  let i = 0;
  let j = 0;
  while (i < m && j < n) {
    if (x[i] === y[j]) {
      mid.push({ kind: " ", text: x[i] });
      i++;
      j++;
    } else if (dp[i + 1][j] >= dp[i][j + 1]) mid.push({ kind: "-", text: x[i++] });
    else mid.push({ kind: "+", text: y[j++] });
  }
  while (i < m) mid.push({ kind: "-", text: x[i++] });
  while (j < n) mid.push({ kind: "+", text: y[j++] });
  return [...a.slice(0, start).map((text) => ({ kind: " " as const, text })), ...mid, ...a.slice(endA).map((text) => ({ kind: " " as const, text }))];
}

function DiffView({ before, after }: { before: string | null; after: string }) {
  const lines = diffLines(before, after);
  if (!lines) {
    return <p className="text-xs text-muted">Too big to compare line by line.</p>;
  }
  // Only the changed parts, with three lines around each.
  const keep = new Set<number>();
  lines.forEach((l, i) => {
    if (l.kind !== " ") for (let k = Math.max(0, i - 3); k <= Math.min(lines.length - 1, i + 3); k++) keep.add(k);
  });
  const rows: (Line | null)[] = [];
  lines.forEach((l, i) => {
    if (keep.has(i)) rows.push(l);
    else if (rows[rows.length - 1] !== null) rows.push(null);
  });
  return (
    <pre className="max-h-96 overflow-auto rounded-lg border border-line bg-deep p-2 font-mono text-xs leading-relaxed">
      {rows.map((l, i) =>
        l === null ? (
          <div key={i} className="text-muted">
            ⋯
          </div>
        ) : (
          <div
            key={i}
            className={l.kind === "+" ? "bg-ok/15 text-ink" : l.kind === "-" ? "bg-danger/15 text-ink line-through decoration-danger/50" : "text-muted"}
          >
            {l.kind} {l.text}
          </div>
        ),
      )}
    </pre>
  );
}

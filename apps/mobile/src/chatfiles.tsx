// Files in chats (phone): the chat's folders (server folders can be added here; PC folders
// from the desktop app), whether it asks before changing files, and each reply's changes to
// apply, skip or undo (server folders; a PC folder's change is applied on that PC).

import { useCallback, useEffect, useState } from "react";
import { Alert, Modal, Pressable, ScrollView, Text, View } from "react-native";
import OutpostNet from "../modules/outpost-net";
import { ApiError, type Outpost } from "./outpost";
import { Button, C, Chip, ErrorText, P, s } from "./ui";

export interface ChatFolder {
  kind: "server" | "pc";
  id: string;
  name: string;
  device?: string | null;
}

export interface FileAction {
  op: string;
  path: string;
  note?: string | null;
  change_id?: string | null;
  failed?: boolean;
}

export interface FileChange {
  id: string;
  kind: "server" | "pc";
  folder: string;
  path: string;
  before: string | null;
  after: string;
  status: "proposed" | "applied" | "skipped" | "undone";
}

export interface Folder {
  id: string;
  name: string;
  /** Its files' total size. */
  bytes?: number;
  /** How many chats use it. */
  chats?: number;
}

/** A size for people: "12 MB". */
export function formatBytes(n: number) {
  if (n >= 1e9) return `${(n / 1e9).toFixed(n >= 1e10 ? 0 : 1)} GB`;
  if (n >= 1e6) return `${Math.round(n / 1e6)} MB`;
  if (n >= 1e3) return `${Math.round(n / 1e3)} KB`;
  return `${n} B`;
}

export function useChatFiles(api: Outpost, chatId: string) {
  const [folders, setFolders] = useState<ChatFolder[]>([]);
  const [changes, setChanges] = useState<FileChange[]>([]);
  const reload = useCallback(async () => {
    const [f, c] = await Promise.all([
      api.get<ChatFolder[]>(`/api/chats/${chatId}/folders`).catch(() => [] as ChatFolder[]),
      api.get<FileChange[]>(`/api/chats/${chatId}/changes`).catch(() => [] as FileChange[]),
    ]);
    setFolders(f);
    setChanges(c);
  }, [api, chatId]);
  useEffect(() => {
    void reload();
  }, [reload]);
  return { folders, setFolders, changes, setChanges, reload };
}

export type ChatFiles = ReturnType<typeof useChatFiles>;

/** The chat's folders and the ask-first switch. */
export function FilesSheet({
  api,
  chatId,
  mode,
  files,
  onMode,
  onClose,
}: {
  api: Outpost;
  chatId: string;
  mode: "ask" | "auto";
  files: ChatFiles;
  onMode: (m: "ask" | "auto") => void;
  onClose: () => void;
}) {
  const [server, setServer] = useState<Folder[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    api.get<Folder[]>("/api/folders").then(setServer, () => undefined);
  }, [api]);
  const save = async (list: ChatFolder[]) => {
    setError(null);
    try {
      files.setFolders(await api.json<ChatFolder[]>("PUT", `/api/chats/${chatId}/folders`, list));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't change the folders.");
    }
  };
  const unused = server.filter((f) => !files.folders.some((x) => x.kind === "server" && x.id === f.id));
  const total = server.reduce((n, f) => n + (f.bytes ?? 0), 0);
  // Delete a server folder and its files (anything running in it stops).
  function removeServerFolder(f: Folder) {
    const here = files.folders.some((x) => x.kind === "server" && x.id === f.id);
    const others = (f.chats ?? 0) - (here ? 1 : 0);
    Alert.alert(
      `Delete "${f.name}" from the server?`,
      `${formatBytes(f.bytes ?? 0)} of files go, and anything running in it stops.${others > 0 ? ` ${others} other chat${others === 1 ? " uses" : "s use"} it too.` : ""}`,
      [
        { text: "Keep", style: "cancel" },
        {
          text: "Delete",
          style: "destructive",
          onPress: () =>
            void api.json("DELETE", `/api/folders/${f.id}`).then(
              () => {
                setServer((list) => list.filter((x) => x.id !== f.id));
                if (here) files.setFolders(files.folders.filter((x) => !(x.kind === "server" && x.id === f.id)));
              },
              (e) => setError(e instanceof Error ? e.message : "Couldn't delete it."),
            ),
        },
      ],
    );
  }
  return (
    <Modal visible transparent animationType="slide" onRequestClose={onClose}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "flex-end" }}>
        <View style={[s.card, { maxHeight: "85%", borderBottomLeftRadius: 0, borderBottomRightRadius: 0 }]}>
          <ScrollView contentContainerStyle={{ gap: 12 }}>
            <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>Files for this chat</Text>
            <P muted>The chat's model can read, search and change files in these folders (it needs a model with the tools badge).</P>
            <View style={s.row}>
              <Chip
                label={mode === "ask" ? "Ask before changing files: on" : "Ask before changing files: off"}
                active={mode === "ask"}
                onPress={() => onMode(mode === "ask" ? "auto" : "ask")}
              />
            </View>
            <P muted>{mode === "ask" ? "Each change waits for you to Apply or Skip it." : "Changes are saved straight away; each has Undo."}</P>
            <Text style={s.label}>Folders</Text>
            {files.folders.length === 0 && <P muted>None yet.</P>}
            {files.folders.map((f) => (
              <View key={`${f.kind}-${f.id}`} style={[s.row, { alignItems: "center" }]}>
                <View style={{ flex: 1 }}>
                  <Text style={{ color: C.ink }}>{f.name}</Text>
                  <Text style={{ color: C.muted, fontSize: 13 }}>
                    {f.kind === "server" ? "On the server" : `On ${f.device ?? "a PC"} (used from the desktop app there)`}
                  </Text>
                  {f.kind === "server" && <Running api={api} folderId={f.id} />}
                </View>
                <Chip label="Remove" onPress={() => void save(files.folders.filter((x) => !(x.kind === f.kind && x.id === f.id)))} />
              </View>
            ))}
            {server.length > 0 && (
              <>
                <Text style={s.label}>
                  Your server folders ({server.length}, {formatBytes(total)})
                </Text>
                {server.map((f) => (
                  <View key={`all-${f.id}`} style={[s.row, { alignItems: "center" }]}>
                    <View style={{ flex: 1 }}>
                      <Text style={{ color: C.ink }}>{f.name}</Text>
                      <Text style={{ color: C.muted, fontSize: 13 }}>
                        {formatBytes(f.bytes ?? 0)} · {f.chats ? `${f.chats} chat${f.chats === 1 ? "" : "s"}` : "no chats"}
                      </Text>
                    </View>
                    <Chip label="Delete" color={C.danger} onPress={() => removeServerFolder(f)} />
                  </View>
                ))}
              </>
            )}
            {unused.length > 0 && (
              <>
                <Text style={s.label}>Add a server folder</Text>
                <View style={[s.row, { flexWrap: "wrap" }]}>
                  {unused.map((f) => (
                    <Chip key={f.id} label={f.name} onPress={() => void save([...files.folders, { kind: "server", id: f.id, name: f.name }])} />
                  ))}
                </View>
              </>
            )}
            <P muted>New server folders, and folders on a PC, are added from the Outpost desktop app (Chat › Files).</P>
            {error && <ErrorText>{error}</ErrorText>}
            <Button title="Done" onPress={onClose} />
          </ScrollView>
        </View>
      </View>
    </Modal>
  );
}

/** What the chat started in a server folder that keeps running (a test server): stop it. */
function Running({ api, folderId }: { api: Outpost; folderId: string }) {
  const [list, setList] = useState<{ name: string; command?: string; running: boolean }[]>([]);
  const look = useCallback(() => {
    api.get<{ name: string; command?: string; running: boolean }[]>(`/api/folders/${folderId}/processes`).then(setList, () => setList([]));
  }, [api, folderId]);
  useEffect(() => {
    look();
    const t = setInterval(look, 5000);
    return () => clearInterval(t);
  }, [look]);
  return (
    <>
      {list
        .filter((p) => p.running)
        .map((p) => (
          <View key={p.name} style={[s.row, { alignItems: "center", marginTop: 4 }]}>
            <Text numberOfLines={1} style={{ color: C.ok, fontSize: 12, fontFamily: "monospace", flex: 1 }}>
              ● {p.name}
              {p.command ? ` · ${p.command}` : ""}
            </Text>
            <Chip
              label="Stop"
              color={C.danger}
              onPress={() =>
                void api
                  .json("POST", `/api/folders/${folderId}/processes/${encodeURIComponent(p.name)}/stop`)
                  .catch(() => undefined)
                  .then(() => setTimeout(look, 800))
              }
            />
          </View>
        ))}
    </>
  );
}

const VERB: Record<string, string> = {
  list: "Looked in",
  read: "Read",
  search: "Searched",
  write: "Wrote",
  edit: "Changed",
  tool: "Tried",
  mcp: "Used",
  run: "Ran in",
  start: "Started in",
  output: "Read the log in",
  input: "Typed into",
  stop: "Stopped in",
  download: "Built",
};

/** A file a build made in a server folder: save it to the phone's Download/Outpost. */
function BuiltFile({ api, action, files }: { api: Outpost; action: FileAction; files: ChatFiles }) {
  const [state, setState] = useState<string | null>(null);
  const [folderName, ...rest] = action.path.split("/");
  const inner = rest.join("/");
  const folder = files.folders.find((f) => f.kind !== "pc" && f.name.toLowerCase() === folderName.toLowerCase());
  const name = rest[rest.length - 1] ?? "build.jar";
  async function save() {
    if (!folder) return;
    setState("Downloading…");
    try {
      const file = await api.download(`/api/folders/${folder.id}/file?path=${encodeURIComponent(inner)}`, `${Date.now()}-${name}`);
      await OutpostNet.saveToPhone(file, name, "application/java-archive");
      setState("Saved to Download/Outpost");
    } catch (e) {
      setState(e instanceof Error ? e.message : "Couldn't save it.");
    }
  }
  return (
    <View style={[s.row, { flexWrap: "wrap" }]}>
      <Text numberOfLines={1} style={{ color: C.ok, fontSize: 12, fontFamily: "monospace", flex: 1 }}>
        Built {action.path}
      </Text>
      {folder && <Chip label="Download" onPress={() => void save()} />}
      {state && <Text style={{ color: C.muted, fontSize: 12 }}>{state}</Text>}
    </View>
  );
}

/** Under a reply: what it did with files, and its changes. */
export function FileActions({ api, chatId, actions, files, live }: { api: Outpost; chatId: string; actions: FileAction[]; files: ChatFiles; live?: boolean }) {
  const [open, setOpen] = useState(false);
  const looked = actions.filter((a) => !a.change_id && !["run", "start", "stop", "download"].includes(a.op));
  const runs = actions.filter((a) => ["run", "start", "stop"].includes(a.op));
  const built = actions.filter((a) => a.op === "download");
  const changed = actions.filter((a) => a.change_id);
  return (
    <View style={[s.card, { gap: 8, paddingVertical: 10 }]}>
      {runs.map((a, i) => (
        <Text key={`run-${i}`} style={{ color: a.failed ? C.danger : C.ok, fontSize: 12, fontFamily: "monospace" }}>
          {VERB[a.op]} {a.path}
          {a.note ? ` · ${a.note}` : ""}
        </Text>
      ))}
      {built.map((a, i) => (
        <BuiltFile key={`built-${i}`} api={api} action={a} files={files} />
      ))}
      {looked.length > 0 && (
        <Pressable accessibilityRole="button" onPress={() => setOpen((o) => !o)}>
          <Text style={{ color: C.muted }}>
            {live ? "Working…" : looked.every((a) => a.op === "mcp") ? `Used tools (${looked.length})` : `Looked at files (${looked.length})`}{" "}
            {open ? "▾" : "▸"}
          </Text>
        </Pressable>
      )}
      {(open || live) &&
        looked.map((a, i) => (
          <Text key={i} numberOfLines={1} style={{ color: a.failed ? C.danger : C.muted, fontSize: 12, fontFamily: "monospace" }}>
            {VERB[a.op] ?? a.op} {a.path}
            {a.note ? ` · ${a.note}` : ""}
          </Text>
        ))}
      {changed.map((a) => (
        <ChangeRow key={a.change_id!} api={api} chatId={chatId} action={a} files={files} />
      ))}
      {changed.length >= 2 && <AllChanges api={api} chatId={chatId} actions={changed} files={files} />}
    </View>
  );
}

/** A reply's changes in server folders, all at once (a PC folder's are done on that PC). */
function AllChanges({ api, chatId, actions, files }: { api: Outpost; chatId: string; actions: FileAction[]; files: ChatFiles }) {
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const mine = actions
    .map((a) => files.changes.find((c) => c.id === a.change_id))
    .filter((c): c is FileChange => !!c && c.kind !== "pc");
  const waiting = mine.filter((c) => c.status === "proposed");
  const applied = mine.filter((c) => c.status === "applied");
  if (waiting.length < 2 && applied.length < 2) return null;
  async function all(what: "apply" | "skip" | "undo") {
    const list = what === "undo" ? [...applied].reverse() : waiting;
    setBusy(true);
    setNote(null);
    let done = 0;
    let left = 0;
    // One that fails doesn't stop the rest: they're listed at the end.
    const failed: string[] = [];
    for (const c of list) {
      try {
        const updated = await api.json<FileChange>("POST", `/api/chats/${chatId}/changes/${c.id}/${what}`, { force: false });
        files.setChanges((cs) => cs.map((x) => (x.id === updated.id ? updated : x)));
        done++;
      } catch (e) {
        if (e instanceof ApiError && e.code === "changed_since") left++;
        else failed.push(`${c.path} (${e instanceof Error ? e.message : "didn't work"})`);
      }
    }
    const parts = [
      left ? `${left} left because the file changed since (do those one by one)` : "",
      failed.length ? `${failed.length} couldn't be done: ${failed.join("; ")}` : "",
    ].filter(Boolean);
    setNote(parts.length ? `${done} done; ${parts.join(". ")}.` : null);
    setBusy(false);
  }
  return (
    <View style={{ gap: 6, borderTopWidth: 1, borderTopColor: C.line, paddingTop: 8 }}>
      <Text style={{ color: C.muted, fontSize: 13 }}>All changes in this reply:</Text>
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {waiting.length >= 2 && <Chip label={busy ? "Working…" : `Apply all (${waiting.length})`} active onPress={() => void all("apply")} />}
        {waiting.length >= 2 && <Chip label="Skip all" onPress={() => void all("skip")} />}
        {applied.length >= 2 && <Chip label={`Undo all (${applied.length})`} onPress={() => void all("undo")} />}
      </View>
      {note && <Text style={{ color: C.amber, fontSize: 13 }}>{note}</Text>}
    </View>
  );
}

function ChangeRow({ api, chatId, action, files }: { api: Outpost; chatId: string; action: FileAction; files: ChatFiles }) {
  const change = files.changes.find((c) => c.id === action.change_id);
  const [show, setShow] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [forceNext, setForceNext] = useState<null | "apply" | "undo">(null);
  const status = change?.status;
  async function act(what: "apply" | "skip" | "undo") {
    if (!change) return;
    const force = forceNext === what;
    setBusy(true);
    setError(null);
    setForceNext(null);
    try {
      const updated = await api.json<FileChange>("POST", `/api/chats/${chatId}/changes/${change.id}/${what}`, { force });
      files.setChanges((cs) => cs.map((c) => (c.id === updated.id ? updated : c)));
    } catch (e) {
      if (e instanceof ApiError && e.code === "changed_since" && what !== "skip") {
        setError(`${e.message} Press ${what === "apply" ? "Apply" : "Undo"} again to do it anyway.`);
        setForceNext(what);
      } else setError(e instanceof Error ? e.message : "That didn't work.");
    } finally {
      setBusy(false);
    }
  }
  const press = (what: "apply" | "skip" | "undo") => void act(what);
  const onPc = change?.kind === "pc";
  return (
    <View style={{ gap: 6 }}>
      <Text style={{ color: action.failed ? C.danger : C.ink, fontSize: 13, fontFamily: "monospace" }}>
        {VERB[action.op] ?? action.op} {action.path}
        {action.note ? `  ${action.note}` : ""}
      </Text>
      {status && (
        <Text style={{ color: status === "proposed" ? C.amber : status === "applied" ? C.ok : C.muted, fontSize: 13 }}>
          {status === "proposed" ? "Waiting for you" : status === "applied" ? "Applied" : status === "skipped" ? "Skipped" : "Undone"}
          {onPc && (status === "proposed" || status === "applied") ? ` · on ${change?.folder}: ${status === "proposed" ? "apply" : "undo"} it from the desktop app on that PC` : ""}
        </Text>
      )}
      <View style={[s.row, { flexWrap: "wrap" }]}>
        {change && <Chip label={show ? "Hide change" : "Show change"} onPress={() => setShow((x) => !x)} />}
        {!onPc && status === "proposed" && (
          <>
            <Chip label="Apply" active onPress={() => press("apply")} />
            <Chip label="Skip" onPress={() => press("skip")} />
          </>
        )}
        {!onPc && status === "applied" && <Chip label="Undo" onPress={() => press("undo")} />}
        {busy && <Text style={{ color: C.muted }}>…</Text>}
      </View>
      {show && change && <Diff before={change.before} after={change.after} />}
      {error && <ErrorText>{error}</ErrorText>}
    </View>
  );
}

/** What changed: removed lines then added ones, around each changed part. */
function Diff({ before, after }: { before: string | null; after: string }) {
  const a = (before ?? "").split("\n");
  const b = after.split("\n");
  let start = 0;
  while (before !== null && start < a.length && start < b.length && a[start] === b[start]) start++;
  let endA = a.length;
  let endB = b.length;
  while (before !== null && endA > start && endB > start && a[endA - 1] === b[endB - 1]) {
    endA--;
    endB--;
  }
  const removed = before === null ? [] : a.slice(start, endA);
  const added = b.slice(start, endB);
  const line = (t: string, kind: "+" | "-", i: number) => (
    <Text
      key={`${kind}${i}`}
      style={{
        color: C.ink,
        backgroundColor: kind === "+" ? "rgba(80,200,120,0.15)" : "rgba(240,90,90,0.15)",
        fontFamily: "monospace",
        fontSize: 11,
      }}
    >
      {kind} {t}
    </Text>
  );
  return (
    <ScrollView horizontal style={{ maxHeight: 360 }}>
      <View>
        {start > 0 && <Text style={{ color: C.muted, fontSize: 11 }}>⋯ line {start + 1}</Text>}
        {removed.slice(0, 200).map((t, i) => line(t, "-", i))}
        {added.slice(0, 200).map((t, i) => line(t, "+", i))}
        {(removed.length > 200 || added.length > 200) && <Text style={{ color: C.muted, fontSize: 11 }}>⋯ more on the PC</Text>}
      </View>
    </ScrollView>
  );
}

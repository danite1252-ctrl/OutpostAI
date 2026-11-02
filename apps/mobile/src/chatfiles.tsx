// Files in chats (phone): the chat's folders (server folders can be added here; PC folders
// from the desktop app), whether it asks before changing files, and each reply's changes to
// apply, skip or undo (server folders; a PC folder's change is applied on that PC).

import { useCallback, useEffect, useState } from "react";
import { Modal, Pressable, ScrollView, Text, View } from "react-native";
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

interface Folder {
  id: string;
  name: string;
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
                </View>
                <Chip label="Remove" onPress={() => void save(files.folders.filter((x) => !(x.kind === f.kind && x.id === f.id)))} />
              </View>
            ))}
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

const VERB: Record<string, string> = { list: "Looked in", read: "Read", search: "Searched", write: "Wrote", edit: "Changed", tool: "Tried", mcp: "Used" };

/** Under a reply: what it did with files, and its changes. */
export function FileActions({ api, chatId, actions, files, live }: { api: Outpost; chatId: string; actions: FileAction[]; files: ChatFiles; live?: boolean }) {
  const [open, setOpen] = useState(false);
  const looked = actions.filter((a) => !a.change_id);
  const changed = actions.filter((a) => a.change_id);
  return (
    <View style={[s.card, { gap: 8, paddingVertical: 10 }]}>
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

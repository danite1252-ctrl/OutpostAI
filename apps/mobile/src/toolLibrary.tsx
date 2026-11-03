// Tools on the phone: your tools on the Outpost server (and the household's shared ones),
// and the library to add more. Tools added here live on the server, so they work in every
// chat, on every device. Signing in: the server does the OAuth work, the phone's browser signs
// in, and comes back to Outpost on this phone (127.0.0.1), which hands it to the server.

import { useCallback, useEffect, useMemo, useState } from "react";
import { Alert, Linking, Modal, Pressable, ScrollView, Text, View } from "react-native";
import OutpostNet from "../modules/outpost-net";
import { ApiError, type Outpost } from "./outpost";
import { Button, C, Chip, ErrorText, Field, P, s } from "./ui";

export interface ServerTool {
  id: string;
  name: string;
  catalog_id?: string | null;
  transport: string;
  command: string;
  args: string[];
  url: string;
  enabled: boolean;
  shared: boolean;
  restricted_ok: boolean;
  mine: boolean;
  owner_name: string | null;
  signed_in: boolean;
  signed_in_as: string | null;
  secret_names: string[];
  disabled_tools: string[];
}

interface CatalogInput {
  key: string;
  label: string;
  kind: "env" | "header" | "arg" | "url" | "client_id" | "client_secret";
  name?: string;
  format?: string;
  args?: string[];
  secret?: boolean;
  optional?: boolean;
  placeholder?: string;
  help?: string;
  link?: string;
  value?: string;
}

export interface CatalogEntry {
  id: string;
  name: string;
  category: string;
  description: string;
  transport: "stdio" | "http" | "builtin";
  command?: string;
  args?: string[];
  url?: string;
  inputs: CatalogInput[];
  cost: string;
  page: string;
  note?: string;
  starter?: boolean;
  signIn?: boolean;
  setup?: string[];
}

const CATEGORIES = ["Search & the web", "Office & files", "Coding", "Knowledge", "Home & tasks", "Helpers"];
/** These work with a computer's own files or browser: added from the desktop app. */
const COMPUTER_ONLY = new Set(["filesystem", "git", "playwright", "google-sheets-sa", "memory"]);

function message(e: unknown) {
  if (e instanceof ApiError || e instanceof Error) return e.message.replace(/^needs (node|uv|sign-in|client): /, "");
  return "Something went wrong.";
}
const needsRuntime = (e: unknown): "node" | "uv" | null => {
  const m = e instanceof Error ? e.message : "";
  return m.startsWith("needs node:") ? "node" : m.startsWith("needs uv:") ? "uv" : null;
};

/** Where the phone catches the browser coming back from a sign-in (the same port the desktop
 *  uses for OAuth clients people make themselves, so one client works for both). */
const SIGN_IN_PORT = 33418;

/** Sign in to one of your server tools in the phone's browser. `client`: an OAuth client of
 *  your own (a service that won't let apps register, or Google's before the household has one). */
export async function signInOnPhone(api: Outpost, id: string, client?: { id?: string; secret?: string }): Promise<ServerTool> {
  if (!OutpostNet.signInListen(SIGN_IN_PORT)) throw new Error("Another sign-in is still open. Wait a moment and try again.");
  try {
    const { url } = await api.json<{ url: string }>("POST", `/api/tools/${id}/sign-in/start`, {
      port: SIGN_IN_PORT,
      client_id: client?.id?.trim() || undefined,
      client_secret: client?.secret?.trim() || undefined,
    });
    await Linking.openURL(url);
    const back = await OutpostNet.signInWait(5 * 60 * 1000);
    return await api.json<ServerTool>("POST", `/api/tools/${id}/sign-in/finish`, { callback: back });
  } finally {
    OutpostNet.signInStop();
  }
}

const needsClient = (e: unknown) => e instanceof ApiError && e.code === "needs_client";

/** A library entry filled in, as the server's add request. */
function toRequest(e: CatalogEntry, values: Record<string, string>) {
  const env: Record<string, string> = {};
  const headers: Record<string, string> = {};
  const args = [...(e.args ?? [])];
  let url = e.url ?? "";
  for (const input of e.inputs) {
    const v = (values[input.key] ?? "").trim();
    if (!v) continue;
    if (input.kind === "env" && input.name) env[input.name] = v;
    else if (input.kind === "header" && input.name) headers[input.name] = (input.format ?? "{value}").replace("{value}", v);
    else if (input.kind === "arg") args.push(...(input.args ?? ["{value}"]).map((a) => a.replace("{value}", v)));
    else if (input.kind === "url") url = url.replace(`{${input.key}}`, v.replace(/\/+$/, ""));
  }
  return { name: e.name, catalog_id: e.id, transport: e.transport, command: e.command ?? "", args, url, secrets: { env, headers } };
}

export function ToolsScreen({ api, isOwner, canAdd, onClose }: { api: Outpost; isOwner: boolean; canAdd: boolean; onClose: () => void }) {
  const [tools, setTools] = useState<ServerTool[] | null>(null);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const reload = useCallback(
    () => api.get<ServerTool[]>("/api/tools").then(setTools, (e) => setError(message(e))),
    [api],
  );
  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <Modal visible animationType="slide" onRequestClose={adding ? () => setAdding(false) : onClose}>
      <View style={[s.screen, { paddingTop: 12 }]}>
        <View style={[s.row, { paddingHorizontal: 16, paddingBottom: 8, alignItems: "center" }]}>
          <Text style={{ color: C.ink, fontSize: 20, fontWeight: "700", flex: 1 }}>{adding ? "Add a tool" : "Tools"}</Text>
          <Chip label={adding ? "‹ Your tools" : "Close"} onPress={adding ? () => setAdding(false) : onClose} />
        </View>
        {adding ? (
          <Library
            api={api}
            isOwner={isOwner}
            onAdded={() => {
              setAdding(false);
              void reload();
            }}
          />
        ) : (
          <ScrollView contentContainerStyle={[s.pad, { gap: 12 }]}>
            <P muted>
              Tools let chats look things up and act for you: search, Google, Home Assistant and more. They run on your Outpost,
              so they work on every device. Pick which ones a chat uses from its + menu.
            </P>
            {canAdd && <Button title="Add a tool" onPress={() => setAdding(true)} />}
            {tools === null && !error && <P muted>Loading…</P>}
            {tools?.length === 0 && <P muted>No tools yet.</P>}
            {tools?.map((t) => <ToolCard key={t.id} api={api} tool={t} isOwner={isOwner} onChanged={() => void reload()} />)}
            <ErrorText>{error}</ErrorText>
          </ScrollView>
        )}
      </View>
    </Modal>
  );
}

function ToolCard({ api, tool, isOwner, onChanged }: { api: Outpost; tool: ServerTool; isOwner: boolean; onChanged: () => void }) {
  const [busy, setBusy] = useState<string | null>(null);
  const [result, setResult] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [runtime, setRuntime] = useState<"node" | "uv" | null>(null);
  const [wantsSignIn, setWantsSignIn] = useState(false);
  const [askClient, setAskClient] = useState(false);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const google = tool.transport === "builtin" && tool.command === "google";

  async function signIn() {
    setBusy("Signing in…");
    setError(null);
    setResult(null);
    try {
      await signInOnPhone(api, tool.id, askClient ? { id: clientId, secret: clientSecret } : undefined);
      setAskClient(false);
      setWantsSignIn(false);
      onChanged();
      setResult("Signed in.");
    } catch (e) {
      if (needsClient(e)) setAskClient(true);
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }
  function signOut() {
    void api.json("DELETE", `/api/tools/${tool.id}/sign-in`).then(onChanged, (e) => setError(message(e)));
  }

  async function change(body: Record<string, unknown>) {
    setError(null);
    try {
      await api.json("PATCH", `/api/tools/${tool.id}`, body);
      onChanged();
    } catch (e) {
      setError(message(e));
    }
  }
  async function test() {
    setBusy("Starting…");
    setError(null);
    setResult(null);
    setRuntime(null);
    try {
      const list = await api.get<{ name: string }[]>(`/api/tools/${tool.id}/tools`);
      setResult(`Working: ${list.length} tools.`);
    } catch (e) {
      setRuntime(needsRuntime(e));
      if (e instanceof Error && e.message.startsWith("needs sign-in:")) setWantsSignIn(true);
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }
  async function getRuntime(which: "node" | "uv") {
    setBusy("Downloading…");
    try {
      await api.json("POST", `/api/tools/runtimes/${which}`);
      // It downloads in the background: check until it's there.
      for (let i = 0; i < 90; i++) {
        await new Promise((r) => setTimeout(r, 2000));
        const now = await api.get<Record<string, unknown>>("/api/tools/runtimes");
        if (now[which]) break;
        if (!now.installing && typeof now.error === "string" && now.error) throw new Error(now.error);
      }
      setBusy(null);
      await test();
    } catch (e) {
      setError(message(e));
      setBusy(null);
    }
  }
  function remove() {
    Alert.alert(`Remove ${tool.name}?`, "Its settings and sign-in on the server are deleted.", [
      { text: "Keep", style: "cancel" },
      {
        text: "Remove",
        style: "destructive",
        onPress: () =>
          void api.json("DELETE", `/api/tools/${tool.id}`).then(onChanged, (e) => setError(message(e))),
      },
    ]);
  }

  const where = google
    ? `Google${tool.signed_in_as ? ` · ${tool.signed_in_as}` : ""}`
    : tool.mine
      ? tool.shared
        ? "Yours · shared with everyone"
        : "Yours"
      : `Shared by ${tool.owner_name ?? "an owner"}`;
  const showSignIn = tool.mine && (google || tool.signed_in || wantsSignIn || askClient);
  return (
    <View style={[s.card, { gap: 8 }]}>
      <View style={[s.row, { alignItems: "center" }]}>
        <View style={{ flex: 1 }}>
          <Text style={{ color: tool.enabled ? C.ink : C.muted, fontWeight: "600" }}>{tool.name}</Text>
          <Text style={{ color: C.muted, fontSize: 13 }}>{where}</Text>
        </View>
        {tool.mine && <Chip label={tool.enabled ? "On" : "Off"} active={tool.enabled} onPress={() => void change({ enabled: !tool.enabled })} />}
      </View>
      {showSignIn && (
        <View style={{ gap: 8 }}>
          <Text style={{ color: tool.signed_in ? C.ok : C.muted, fontSize: 13 }}>
            {tool.signed_in
              ? `Signed in${tool.signed_in_as ? ` as ${tool.signed_in_as}` : ""}`
              : `Sign in to ${tool.name}: the phone's browser opens, then comes back here. The sign-in is kept on your Outpost, for all your devices.`}
          </Text>
          {askClient && (
            <>
              <Text style={{ color: C.soft, fontSize: 13 }}>
                {google
                  ? "Enter the household's Google OAuth client (made once, as the Google tool's steps say)."
                  : "This service doesn't let apps register themselves: make an OAuth client for it (its instructions say how), with the redirect address http://127.0.0.1:33418/callback."}
              </Text>
              <Field label="OAuth client ID" value={clientId} onChangeText={setClientId} autoCapitalize="none" autoCorrect={false} />
              <Field label="Client secret (if it gave one)" value={clientSecret} onChangeText={setClientSecret} secureTextEntry autoCapitalize="none" autoCorrect={false} />
            </>
          )}
          <View style={[s.row, { flexWrap: "wrap" }]}>
            <Chip
              label={busy === "Signing in…" ? "Waiting for the browser…" : tool.signed_in ? "Sign in again" : "Sign in"}
              active={!tool.signed_in}
              onPress={() => void signIn()}
            />
            {tool.signed_in && <Chip label="Sign out" onPress={signOut} />}
          </View>
        </View>
      )}
      <View style={[s.row, { flexWrap: "wrap" }]}>
        <Chip label={busy === "Starting…" ? busy : "Test"} onPress={() => void test()} />
        {tool.mine && isOwner && <Chip label={tool.shared ? "Shared" : "Share with everyone"} active={tool.shared} onPress={() => void change({ shared: !tool.shared })} />}
        {tool.mine && isOwner && tool.shared && (
          <Chip label="Children's accounts too" active={tool.restricted_ok} onPress={() => void change({ restricted_ok: !tool.restricted_ok })} />
        )}
        {tool.mine && <Chip label="Remove" color={C.danger} onPress={remove} />}
      </View>
      {result && <Text style={{ color: C.ok, fontSize: 13 }}>{result}</Text>}
      {error && <Text style={{ color: C.danger, fontSize: 13 }}>{error}</Text>}
      {runtime && isOwner && (
        <Button
          kind="ghost"
          title={busy === "Downloading…" ? `Downloading ${runtime === "node" ? "Node.js" : "uv"} on the server…` : `Get ${runtime === "node" ? "Node.js" : "Python (uv)"} on the server`}
          busy={busy === "Downloading…"}
          onPress={() => void getRuntime(runtime)}
        />
      )}
    </View>
  );
}

function Library({ api, isOwner, onAdded }: { api: Outpost; isOwner: boolean; onAdded: () => void }) {
  const [entries, setEntries] = useState<CatalogEntry[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [open, setOpen] = useState<CatalogEntry | null>(null);
  useEffect(() => {
    api.get<CatalogEntry[]>("/api/tools/catalog").then(setEntries, (e) =>
      setError(e instanceof ApiError && e.status === 404 ? "Update the Outpost server to add tools from the phone." : message(e)),
    );
  }, [api]);
  const shown = useMemo(() => {
    const q = query.trim().toLowerCase();
    return (entries ?? [])
      .filter((e) => !category || e.category === category)
      .filter((e) => !q || `${e.name} ${e.description} ${e.category}`.toLowerCase().includes(q))
      .sort((a, b) => Number(!!b.starter) - Number(!!a.starter));
  }, [entries, query, category]);

  if (open) return <AddEntry api={api} entry={open} isOwner={isOwner} onBack={() => setOpen(null)} onAdded={onAdded} />;
  return (
    <ScrollView contentContainerStyle={[s.pad, { gap: 12 }]} keyboardShouldPersistTaps="handled">
      <Field placeholder="Search tools" value={query} onChangeText={setQuery} autoCorrect={false} />
      <View style={[s.row, { flexWrap: "wrap" }]}>
        <Chip label="All" active={!category} onPress={() => setCategory(null)} />
        {CATEGORIES.map((c) => (
          <Chip key={c} label={c} active={category === c} onPress={() => setCategory(category === c ? null : c)} />
        ))}
      </View>
      {entries === null && !error && <P muted>Loading the library…</P>}
      <ErrorText>{error}</ErrorText>
      {shown.map((e) => (
        <Pressable key={e.id} accessibilityRole="button" onPress={() => setOpen(e)} style={[s.card, { gap: 4 }]}>
          <Text style={{ color: C.ink, fontWeight: "600" }}>
            {e.name}
            {e.starter ? <Text style={{ color: C.ember, fontSize: 12, fontWeight: "400" }}>  Good first tool</Text> : null}
          </Text>
          <Text style={{ color: C.soft, fontSize: 14 }}>{e.description}</Text>
          <Text style={{ color: C.muted, fontSize: 12 }}>{e.cost}</Text>
        </Pressable>
      ))}
    </ScrollView>
  );
}

function AddEntry({
  api,
  entry,
  isOwner,
  onBack,
  onAdded,
}: {
  api: Outpost;
  entry: CatalogEntry;
  isOwner: boolean;
  onBack: () => void;
  onAdded: () => void;
}) {
  const google = entry.transport === "builtin" && entry.command === "google";
  const [values, setValues] = useState<Record<string, string>>(() =>
    Object.fromEntries(entry.inputs.filter((i) => i.value).map((i) => [i.key, i.value!])),
  );
  // Google's client is set once for the household; when it is, there's nothing to fill in.
  const [householdClient, setHouseholdClient] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  useEffect(() => {
    if (!google) return;
    api.get<{ client_id: string | null }>("/api/tools/google-client").then((g) => setHouseholdClient(g.client_id || null), () => undefined);
  }, [api, google]);

  const program = entry.transport === "stdio";
  const computerOnly = COMPUTER_ONLY.has(entry.id);
  const blocked = computerOnly
    ? "This one works with a computer's own files or browser: add it from the desktop app on that PC."
    : program && !isOwner
      ? "This one runs a program on the server, so only an owner can add it. Ask an owner to add and share it."
      : google && !householdClient && !isOwner
        ? "An owner needs to set up the household's Google client first (once, from the desktop app or here)."
        : null;
  const inputs = entry.inputs.filter((i) => !(google && householdClient && (i.kind === "client_id" || i.kind === "client_secret")));
  const missing = inputs.filter((i) => !i.optional && !(values[i.key] ?? "").trim()).map((i) => i.label);

  async function add() {
    setBusy(true);
    setError(null);
    try {
      if (google && !householdClient && values.client_id?.trim()) {
        await api.json("PUT", "/api/tools/google-client", {
          client_id: values.client_id.trim(),
          client_secret: values.client_secret?.trim() || undefined,
        });
      }
      const tool = await api.json<ServerTool>("POST", "/api/tools", toRequest(entry, values));
      if (entry.signIn || google) {
        // Straight on to signing in (the client typed here, for services that need one).
        const client = { id: values.client_id, secret: values.client_secret };
        try {
          await signInOnPhone(api, tool.id, client.id ? client : undefined);
          setDone(`${entry.name} is added and signed in. Turn it on in a chat from its + menu › Tools.`);
        } catch (e) {
          setDone(`${entry.name} is added, but not signed in: ${message(e)} Sign in under Your tools.`);
        }
        return;
      }
      try {
        const list = await api.get<{ name: string }[]>(`/api/tools/${tool.id}/tools`);
        setDone(`${entry.name} is added and working (${list.length} tools). Turn it on in a chat from its + menu › Tools.`);
      } catch (e) {
        setDone(`${entry.name} is added, but it didn't start: ${message(e)} Open it under Your tools to test it again.`);
      }
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <ScrollView contentContainerStyle={[s.pad, { gap: 12 }]} keyboardShouldPersistTaps="handled">
      <Text style={{ color: C.ink, fontSize: 18, fontWeight: "700" }}>{entry.name}</Text>
      <P>{entry.description}</P>
      <P muted>{entry.cost}</P>
      {entry.note && <P muted>{entry.note}</P>}
      <Pressable accessibilityRole="link" onPress={() => void Linking.openURL(entry.page)}>
        <Text style={{ color: C.pick }}>About it</Text>
      </Pressable>
      {done ? (
        <>
          <P>{done}</P>
          <Button title="Done" onPress={onAdded} />
        </>
      ) : blocked ? (
        <>
          <P muted>{blocked}</P>
          <Button kind="ghost" title="Back to the library" onPress={onBack} />
        </>
      ) : (
        <>
          {entry.setup && !(google && householdClient) && (
            <View style={[s.card, { gap: 6 }]}>
              <Text style={{ color: C.ink, fontWeight: "600" }}>First, once</Text>
              {entry.setup.map((step, i) => (
                <Text key={i} style={{ color: C.soft, fontSize: 14 }}>
                  {i + 1}. {step}
                </Text>
              ))}
            </View>
          )}
          {inputs.map((i) => (
            <View key={i.key} style={{ gap: 4 }}>
              <Field
                label={`${i.label}${i.optional ? " (optional)" : ""}`}
                placeholder={i.placeholder}
                value={values[i.key] ?? ""}
                onChangeText={(v) => setValues((cur) => ({ ...cur, [i.key]: v }))}
                secureTextEntry={!!i.secret}
                autoCapitalize="none"
                autoCorrect={false}
              />
              {i.help && <Text style={{ color: C.muted, fontSize: 13 }}>{i.help}</Text>}
              {i.link && (
                <Pressable accessibilityRole="link" onPress={() => void Linking.openURL(i.link!)}>
                  <Text style={{ color: C.pick, fontSize: 13 }}>Where to get it</Text>
                </Pressable>
              )}
            </View>
          ))}
          {program && <P muted>It runs on the server (Outpost downloads Node.js or Python there if it needs to).</P>}
          <ErrorText>{error}</ErrorText>
          <Button
            title={missing.length ? `Fill in: ${missing.join(", ")}` : `Add ${entry.name}`}
            disabled={missing.length > 0}
            busy={busy}
            onPress={() => void add()}
          />
          <Button kind="ghost" title="Back to the library" onPress={onBack} />
        </>
      )}
    </ScrollView>
  );
}

// The Outpost server, for owners on the phone: its version and updates (check, install,
// automatic), a restart, and crash reports to know about.

import { useCallback, useEffect, useState } from "react";
import { Alert, Text, View } from "react-native";
import { message, Toggle, usePin } from "./admin";
import { formatBytes } from "./chatfiles";
import type { Outpost } from "./outpost";
import { Button, C, ErrorText, P, s } from "./ui";

interface UpdateStatus {
  current: string;
  enabled: boolean;
  checked_at: number | null;
  update: { version: string; notes: string | null } | null;
  server_can_install?: boolean;
  installing?: string | null;
  auto_install?: boolean;
  error: string | null;
}
interface Diagnostics {
  crashes: { id: string; kind: string; at: number; summary: string }[];
  unseen: number;
}

export function ServerCard({ api }: { api: Outpost }) {
  const [st, setSt] = useState<UpdateStatus | null>(null);
  const [diag, setDiag] = useState<Diagnostics | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showCrashes, setShowCrashes] = useState(false);
  const [cache, setCache] = useState<{ bytes: number; tools_bytes: number } | null>(null);
  const { guard, prompt } = usePin();

  const reload = useCallback(async () => {
    try {
      setSt(await api.get<UpdateStatus>("/api/updates"));
      setDiag(await api.get<Diagnostics>("/api/diagnostics").catch(() => null));
      setCache(await api.get<{ bytes: number; tools_bytes: number }>("/api/system/build-cache").catch(() => null));
    } catch (e) {
      setError(message(e));
    }
  }, [api]);
  useEffect(() => {
    void reload();
    // While installing or restarting, follow it.
    const t = setInterval(() => void reload(), busy || st?.installing ? 3000 : 30000);
    return () => clearInterval(t);
  }, [reload, busy, st?.installing]);

  async function run(label: string, work: () => Promise<unknown>) {
    setBusy(label);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
      void reload();
    }
  }
  const check = () => run("Checking…", () => api.json<UpdateStatus>("POST", "/api/updates/check").then(setSt));
  /** Something running a restart would stop (a reply being written, a build...): ask first. */
  async function okToRestart(what: string, go: () => void) {
    const busy = await api.get<{ summary: string | null }>("/api/system/busy").catch(() => null);
    if (!busy?.summary) return go();
    Alert.alert(`${what} anyway?`, `${busy.summary} on the server. A restart stops it.`, [
      { text: "Wait", style: "cancel" },
      { text: what, style: "destructive", onPress: go },
    ]);
  }
  const install = () =>
    void okToRestart("Update", () => void run("Installing…", () => guard((h) => api.json("POST", "/api/updates/install", undefined, h))));
  const setAuto = (on: boolean) =>
    run("Saving…", () => api.json<UpdateStatus>("PUT", "/api/updates/settings", { enabled: st?.enabled ?? true, auto_install: on }).then(setSt));
  async function restart() {
    const busy = await api.get<{ summary: string | null }>("/api/system/busy").catch(() => null);
    const now = busy?.summary ? `${busy.summary} now, and will be stopped.` : "Nothing is running now.";
    Alert.alert("Restart the Outpost server?", `${now} It's back in a few seconds.`, [
      { text: "Cancel", style: "cancel" },
      { text: "Restart", style: "destructive", onPress: () => void run("Restarting…", () => guard((h) => api.json("POST", "/api/system/restart", undefined, h))) },
    ]);
  }
  const markSeen = () => run("Saving…", () => api.json("POST", "/api/diagnostics/seen"));

  const installing = st?.installing && !st.installing.startsWith("Failed") ? st.installing : null;
  return (
    <View style={s.card}>
      {prompt}
      <Text style={{ color: C.ink, fontSize: 16, fontWeight: "600" }}>Outpost server</Text>
      {st ? (
        <>
          <P muted>
            Version {st.current}
            {st.update ? ` · ${st.update.version} is out` : " · up to date"}
            {st.checked_at ? ` · checked ${new Date(st.checked_at * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" })}` : ""}
          </P>
          {installing && <Text style={{ color: C.amber }}>Installing {st.update?.version ?? "the update"}: {installing}</Text>}
          {st.installing?.startsWith("Failed") && <ErrorText>{st.installing}</ErrorText>}
          {st.error && <ErrorText>{st.error}</ErrorText>}
          {st.update && st.server_can_install && !installing && (
            <Button title={`Install ${st.update.version} now`} busy={busy === "Installing…"} onPress={() => void install()} />
          )}
          <Button kind="ghost" title={busy === "Checking…" ? "Checking…" : "Check for updates"} busy={busy === "Checking…"} onPress={() => void check()} />
          {st.server_can_install !== undefined && (
            <Toggle
              title="Install server updates automatically"
              hint="As soon as nothing is running (it checks every minute)."
              on={st.auto_install ?? true}
              onChange={(on) => void setAuto(on)}
            />
          )}
        </>
      ) : (
        <P muted>Loading…</P>
      )}
      {diag && diag.crashes.length > 0 && (
        <View style={{ gap: 6 }}>
          <Text style={{ color: diag.unseen ? C.amber : C.muted }} onPress={() => setShowCrashes((x) => !x)}>
            {diag.unseen ? `${diag.unseen} new crash report${diag.unseen === 1 ? "" : "s"}` : `${diag.crashes.length} crash reports`} ·{" "}
            {showCrashes ? "hide" : "show"}
          </Text>
          {showCrashes &&
            diag.crashes.slice(0, 10).map((c) => (
              <Text key={c.id} style={{ color: C.soft, fontSize: 13 }}>
                {new Date(c.at * 1000).toLocaleString()} · {c.kind === "unclean" ? "didn't close normally" : c.summary}
              </Text>
            ))}
          {diag.unseen > 0 && <Button kind="ghost" title="Mark as seen" onPress={() => void markSeen()} />}
          <P muted>Full reports are in Settings › Diagnostics on the PC.</P>
        </View>
      )}
      {cache && (
        <View style={{ gap: 6 }}>
          <P muted>
            Build caches (what coding builds download): {formatBytes(cache.bytes)}
            {cache.tools_bytes > 0 ? `. Java and Gradle, kept: ${formatBytes(cache.tools_bytes)}.` : "."}
          </P>
          {cache.bytes > 0 && (
            <Button
              kind="ghost"
              title={busy === "Clearing…" ? "Clearing…" : "Clear build caches"}
              onPress={() =>
                Alert.alert("Clear the build caches?", `${formatBytes(cache.bytes)} goes. The next build downloads what it needs again.`, [
                  { text: "Keep", style: "cancel" },
                  { text: "Clear", style: "destructive", onPress: () => void run("Clearing…", () => guard((h) => api.json("DELETE", "/api/system/build-cache", undefined, h))) },
                ])
              }
            />
          )}
        </View>
      )}
      <Button kind="danger" title={busy === "Restarting…" ? "Restarting…" : "Restart the server"} onPress={restart} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

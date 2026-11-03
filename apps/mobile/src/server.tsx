// The Outpost server, for owners on the phone: its version and updates (check, install,
// automatic), a restart, and crash reports to know about.

import { useCallback, useEffect, useState } from "react";
import { Alert, Text, View } from "react-native";
import { message, Toggle, usePin } from "./admin";
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
  const { guard, prompt } = usePin();

  const reload = useCallback(async () => {
    try {
      setSt(await api.get<UpdateStatus>("/api/updates"));
      setDiag(await api.get<Diagnostics>("/api/diagnostics").catch(() => null));
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
  const install = () => run("Installing…", () => guard((h) => api.json("POST", "/api/updates/install", undefined, h)));
  const setAuto = (on: boolean) =>
    run("Saving…", () => api.json<UpdateStatus>("PUT", "/api/updates/settings", { enabled: st?.enabled ?? true, auto_install: on }).then(setSt));
  function restart() {
    Alert.alert("Restart the Outpost server?", "Chats and jobs running now are stopped. It's back in a few seconds.", [
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
      <Button kind="danger" title={busy === "Restarting…" ? "Restarting…" : "Restart the server"} onPress={restart} />
      <ErrorText>{error}</ErrorText>
    </View>
  );
}

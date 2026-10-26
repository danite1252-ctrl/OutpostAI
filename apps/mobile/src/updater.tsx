/**
 * Updating the phone app from inside it. The PC checks GitHub Releases each day; when it
 * knows of a newer phone app, this downloads it in the background and installs it:
 * straight away when you press Restart, or (once Android allows updates without a prompt)
 * by itself the next time you leave the app.
 */
import { useCallback, useEffect, useRef, useState } from "react";
import { AppState, Pressable, Text, View } from "react-native";
import OutpostNet, { type UpdateEvent } from "../modules/outpost-net";
import type { Outpost } from "./outpost";
import { loadAutoUpdate } from "./store";
import { C } from "./ui";
import { APP_VERSION } from "./version";

/** `a` is a newer version than `b` (0.10.0 > 0.9.2). */
export function isNewer(a: string, b: string) {
  const parts = (v: string) => v.replace(/^v/, "").split("-")[0].split(".").map((n) => parseInt(n, 10) || 0);
  const x = parts(a);
  const y = parts(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  }
  return false;
}

type Mobile = { version: string; url: string; sha256?: string | null };

export type UpdateState =
  | { phase: "none" }
  /** Newer app out, not downloaded (automatic updates are off). */
  | { phase: "available"; version: string }
  | { phase: "downloading"; version: string; percent: number }
  | { phase: "ready"; version: string; silent: boolean }
  /** Android needs "Install unknown apps" allowed for Outpost first. */
  | { phase: "permission"; version: string }
  | { phase: "installing"; version: string }
  | { phase: "error"; version: string; message: string };

const SIX_HOURS = 6 * 60 * 60 * 1000;

export function usePhoneUpdate(api: Outpost) {
  const [state, setState] = useState<UpdateState>({ phase: "none" });
  const latest = useRef<Mobile | null>(null);
  const path = useRef<string | null>(null);
  const busy = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  const prepare = useCallback(
    async (download: boolean) => {
      if (busy.current) return;
      busy.current = true;
      try {
        const u = await api.get<{ mobile: Mobile | null }>("/api/updates");
        const m = u.mobile;
        if (!m || !isNewer(m.version, APP_VERSION)) {
          setState({ phase: "none" });
          return;
        }
        latest.current = m;
        const name = `Outpost-${m.version}.apk`;
        let file = OutpostNet.updateDownloaded(name);
        if (!file) {
          if (!download && !(await loadAutoUpdate())) {
            setState({ phase: "available", version: m.version });
            return;
          }
          setState({ phase: "downloading", version: m.version, percent: 0 });
          file = await OutpostNet.downloadUpdate(m.url, name, m.sha256 ?? null);
        }
        path.current = file;
        const st = OutpostNet.updateStatus();
        if (!st.canInstall) setState({ phase: "permission", version: m.version });
        else setState({ phase: "ready", version: m.version, silent: st.silent });
      } catch (e) {
        const v = latest.current?.version;
        if (v) setState({ phase: "error", version: v, message: e instanceof Error ? e.message : "The update couldn't be downloaded." });
      } finally {
        busy.current = false;
      }
    },
    [api],
  );

  const install = useCallback(async () => {
    const s = stateRef.current;
    if (!path.current || (s.phase !== "ready" && s.phase !== "error")) return;
    if (!OutpostNet.updateStatus().canInstall) {
      setState({ phase: "permission", version: s.version });
      return;
    }
    setState({ phase: "installing", version: s.version });
    try {
      await OutpostNet.installUpdate(path.current);
    } catch (e) {
      setState({ phase: "error", version: s.version, message: e instanceof Error ? e.message : "The update didn't install." });
    }
  }, []);

  // Android's answers while installing.
  useEffect(() => {
    const sub = OutpostNet.addListener("onUpdate", (e: UpdateEvent) => {
      const s = stateRef.current;
      if (s.phase === "none") return;
      if (e.type === "progress" && s.phase === "downloading") setState({ ...s, percent: Math.max(0, e.percent) });
      else if (e.type === "cancelled") setState({ phase: "ready", version: s.version, silent: false });
      else if (e.type === "failed") setState({ phase: "error", version: s.version, message: e.message });
    });
    return () => sub.remove();
  }, []);

  // Look when the app opens, comes back, and every few hours; install on the way out when
  // Android lets it go in without a prompt and automatic updates are on.
  useEffect(() => {
    void prepare(false);
    const t = setInterval(() => void prepare(false), SIX_HOURS);
    const sub = AppState.addEventListener("change", (st) => {
      const s = stateRef.current;
      if (st === "active") {
        if (s.phase === "permission" || s.phase === "none" || s.phase === "available") void prepare(false);
      } else if (st === "background" && s.phase === "ready" && s.silent) {
        void loadAutoUpdate().then((auto) => auto && void install());
      }
    });
    return () => {
      clearInterval(t);
      sub.remove();
    };
  }, [prepare, install]);

  return { state, install, download: () => void prepare(true), allow: () => OutpostNet.openInstallSettings() };
}

/** A slim bar under the connection banner while an update needs you. */
export function UpdateBar({ update }: { update: ReturnType<typeof usePhoneUpdate> }) {
  const s = update.state;
  if (s.phase === "none" || s.phase === "downloading" || s.phase === "available") return null;
  const text =
    s.phase === "ready"
      ? `Outpost ${s.version} is ready.`
      : s.phase === "permission"
        ? `Outpost ${s.version} is ready. Allow Outpost to install its updates (once).`
        : s.phase === "installing"
          ? `Installing Outpost ${s.version}…`
          : `Outpost ${s.version}: ${s.message}`;
  const action =
    s.phase === "ready" || s.phase === "error"
      ? { label: s.phase === "error" ? "Try again" : "Restart to update", run: () => void update.install() }
      : s.phase === "permission"
        ? { label: "Allow", run: update.allow }
        : null;
  return (
    <View
      accessibilityLiveRegion="polite"
      style={{ flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 14, paddingVertical: 8, backgroundColor: C.raised, borderBottomWidth: 1, borderBottomColor: C.line }}
    >
      <Text style={{ flex: 1, color: s.phase === "error" ? C.danger : C.ink, fontSize: 14 }}>{text}</Text>
      {action && (
        <Pressable accessibilityRole="button" onPress={action.run} hitSlop={6} style={{ paddingHorizontal: 12, paddingVertical: 6, borderRadius: 10, backgroundColor: C.ember }}>
          <Text style={{ color: C.onEmber, fontWeight: "700", fontSize: 14 }}>{action.label}</Text>
        </Pressable>
      )}
    </View>
  );
}

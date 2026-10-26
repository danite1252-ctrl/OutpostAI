import * as LocalAuthentication from "expo-local-authentication";
import { StatusBar } from "expo-status-bar";
import { useCallback, useEffect, useMemo, useState } from "react";
import { AppState, Keyboard, KeyboardAvoidingView, Pressable, Text, View } from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { ApiError, Outpost } from "./src/outpost";
import Chat from "./src/screens/Chat";
import Create from "./src/screens/Create";
import Gallery from "./src/screens/Gallery";
import Me from "./src/screens/Me";
import Pair from "./src/screens/Pair";
import Queue, { useLine } from "./src/screens/Queue";
import { awayFor, forget, load, markAway, save, type Saved } from "./src/store";

/** Unlock again after the app has been in the background this long. */
const LOCK_AFTER_MS = 5 * 60 * 1000;
import { Button, C, H1, P, s } from "./src/ui";

type Tab = "chat" | "create" | "gallery" | "queue" | "me";

const TABS: { id: Tab; label: string }[] = [
  { id: "chat", label: "Chat" },
  { id: "create", label: "Create" },
  { id: "gallery", label: "Gallery" },
  { id: "queue", label: "Queue" },
  { id: "me", label: "You" },
];

export default function App() {
  const [saved, setSaved] = useState<Saved | null | undefined>(undefined);
  const [unlocked, setUnlocked] = useState(false);

  useEffect(() => {
    void Promise.all([load(), awayFor()]).then(([sv, ms]) => {
      // Reopened soon after leaving: no need to unlock again.
      if (sv && ms <= LOCK_AFTER_MS) setUnlocked(true);
      setSaved(sv);
    });
  }, []);

  // Lock only after the app has been away for a while; a quick look elsewhere
  // comes straight back to where you were.
  useEffect(() => {
    const sub = AppState.addEventListener("change", (st) => {
      if (st === "background") void markAway();
      if (st === "active") {
        void awayFor().then((ms) => {
          if (ms > LOCK_AFTER_MS) setUnlocked(false);
        });
      }
    });
    return () => sub.remove();
  }, []);

  const paired = useCallback(async (s: Saved) => {
    await save(s);
    setSaved(s);
    setUnlocked(true);
  }, []);

  const signOut = useCallback(async () => {
    await forget();
    setSaved(null);
    setUnlocked(false);
  }, []);

  let body;
  if (saved === undefined) body = <View style={s.screen} />;
  else if (saved === null) body = <Pair onPaired={(x) => void paired(x)} />;
  else if (!unlocked) body = <Unlock saved={saved} onUnlocked={() => setUnlocked(true)} />;
  else body = <Main saved={saved} onSaved={(x) => void save(x)} onSignedOut={() => void signOut()} />;

  return (
    <SafeAreaProvider>
      <StatusBar style="light" />
      <SafeAreaView style={s.screen} edges={["top", "bottom"]}>
        {/* Android draws edge to edge, so the screen doesn't shrink for the keyboard by
            itself: pad it, so whatever has focus stays above the keyboard. */}
        <KeyboardAvoidingView style={{ flex: 1 }} behavior="padding">
          {body}
        </KeyboardAvoidingView>
      </SafeAreaView>
    </SafeAreaProvider>
  );
}

/** The phone's own fingerprint, face or screen lock opens the app; no PIN each time. */
function Unlock({ saved, onUnlocked }: { saved: Saved; onUnlocked: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const tryUnlock = useCallback(async () => {
    setError(null);
    const has = (await LocalAuthentication.hasHardwareAsync()) && (await LocalAuthentication.isEnrolledAsync());
    if (!has) return onUnlocked();
    const r = await LocalAuthentication.authenticateAsync({ promptMessage: "Unlock Outpost" });
    if (r.success) onUnlocked();
    else setError("Not unlocked. Try again.");
  }, [onUnlocked]);
  useEffect(() => {
    void tryUnlock();
  }, [tryUnlock]);
  return (
    <View style={[s.screen, s.pad, { justifyContent: "center" }]}>
      <H1>Outpost</H1>
      <P muted>
        {saved.account.name} on {saved.name}
      </P>
      <Button title="Unlock" onPress={() => void tryUnlock()} />
      {error && <P muted>{error}</P>}
    </View>
  );
}

function Main({ saved, onSaved, onSignedOut }: { saved: Saved; onSaved: (s: Saved) => void; onSignedOut: () => void }) {
  const [tab, setTab] = useState<Tab>("chat");
  const api = useMemo(() => new Outpost({ ...saved }), [saved]);
  const [revoked, setRevoked] = useState(false);
  const line = useLine(api, tab === "queue");
  const [editImage, setEditImage] = useState<{ dataUri: string; task: "edit" | "animate" } | null>(null);
  // The tab bar steps aside while typing, leaving the room to the text box.
  const [typing, setTyping] = useState(false);
  useEffect(() => {
    const show = Keyboard.addListener("keyboardDidShow", () => setTyping(true));
    const hide = Keyboard.addListener("keyboardDidHide", () => setTyping(false));
    return () => {
      show.remove();
      hide.remove();
    };
  }, []);

  // Learn any new addresses (e.g. Tailscale for away from home) without pairing again.
  useEffect(() => {
    api.get<{ hosts: string[] }>("/api/connection").then(
      (info) => {
        const known = api.conn.hosts;
        const merged = [...known, ...info.hosts.filter((h) => !known.includes(h))];
        if (merged.length !== known.length) {
          api.conn.hosts = merged;
          onSaved({ ...saved, hosts: merged });
        }
      },
      () => undefined,
    );
  }, [api]); // eslint-disable-line react-hooks/exhaustive-deps

  // Remember an address that answered better than the saved one.
  useEffect(() => {
    const t = setInterval(() => {
      if (api.conn.hosts[0] !== saved.hosts[0]) onSaved({ ...saved, hosts: api.conn.hosts });
    }, 10000);
    return () => clearInterval(t);
  }, [api, saved, onSaved]);

  // Back online after a drop: reload the screen so it isn't left empty.
  const [reconnects, setReconnects] = useState(0);
  useEffect(
    () =>
      api.onStatus((on) => {
        if (on) setReconnects((n) => n + 1);
      }),
    [api],
  );

  const onError = useCallback((e: unknown) => {
    if (e instanceof ApiError && e.status === 401) setRevoked(true);
  }, []);

  if (revoked) {
    return (
      <View style={[s.screen, s.pad, { justifyContent: "center" }]}>
        <H1>Signed out</H1>
        <P muted>This phone was unpaired on the Outpost PC, or its PIN changed. Pair it again to carry on.</P>
        <Button title="Pair again" onPress={onSignedOut} />
      </View>
    );
  }

  const mine = line.filter((e) => e.mine).length;
  return (
    <View style={s.screen}>
      <Connection api={api} />
      <View style={{ flex: 1 }} key={reconnects}>
        {tab === "chat" && <Chat api={api} onError={onError} />}
        {tab === "create" && (
          <Create api={api} onError={onError} incoming={editImage} onIncomingUsed={() => setEditImage(null)} onQueued={() => setTab("queue")} />
        )}
        {tab === "gallery" && (
          <Gallery
            api={api}
            me={saved.account}
            onError={onError}
            onUse={(dataUri, task) => {
              setEditImage({ dataUri, task });
              setTab("create");
            }}
          />
        )}
        {tab === "queue" && <Queue api={api} line={line} onError={onError} />}
        {tab === "me" && <Me api={api} saved={saved} onSignedOut={onSignedOut} />}
      </View>
      <View style={{ flexDirection: "row", borderTopWidth: 1, borderTopColor: C.line, backgroundColor: C.surface, display: typing ? "none" : "flex" }}>
        {TABS.map((t) => (
          <Pressable
            key={t.id}
            accessibilityRole="tab"
            accessibilityState={{ selected: tab === t.id }}
            onPress={() => setTab(t.id)}
            style={{ flex: 1, minHeight: 56, alignItems: "center", justifyContent: "center" }}
          >
            <Text style={{ color: tab === t.id ? C.ember : C.muted, fontWeight: tab === t.id ? "700" : "500" }}>
              {t.label}
              {t.id === "queue" && line.length > 0 ? ` ${line.length}` : ""}
            </Text>
            {t.id === "queue" && line.length > 0 && (
              <View style={{ width: 6, height: 6, borderRadius: 3, marginTop: 3, backgroundColor: mine ? C.ember : C.pick }} />
            )}
          </Pressable>
        ))}
      </View>
    </View>
  );
}

/** A banner while the Outpost can't be reached; it checks again by itself. */
function Connection({ api }: { api: Outpost }) {
  const [online, setOnline] = useState(api.online);
  const [checking, setChecking] = useState(false);
  useEffect(() => api.onStatus(setOnline), [api]);
  // Keep an eye on it: quickly while lost, now and then while fine (catches Wi-Fi going off).
  useEffect(() => {
    const t = setInterval(() => void api.ping(), online ? 30000 : 5000);
    return () => clearInterval(t);
  }, [api, online]);
  // Coming back to the app: check straight away.
  useEffect(() => {
    const sub = AppState.addEventListener("change", (st) => {
      if (st === "active") void api.ping();
    });
    return () => sub.remove();
  }, [api]);
  if (online) return null;
  return (
    <View accessibilityRole="alert" style={{ backgroundColor: "#5a1f28", paddingHorizontal: 16, paddingVertical: 10, flexDirection: "row", alignItems: "center", gap: 10 }}>
      <View style={{ flex: 1 }}>
        <Text style={{ color: C.ink, fontWeight: "700" }}>Lost connection to Outpost</Text>
        <Text style={{ color: C.soft, fontSize: 13 }}>Check the phone's Wi-Fi and that the PC is on. Trying again…</Text>
      </View>
      <Pressable
        accessibilityRole="button"
        disabled={checking}
        onPress={() => {
          setChecking(true);
          void api.ping().finally(() => setChecking(false));
        }}
        style={{ borderWidth: 1, borderColor: C.soft, borderRadius: 10, paddingHorizontal: 12, minHeight: 40, justifyContent: "center" }}
      >
        <Text style={{ color: C.ink }}>{checking ? "…" : "Retry"}</Text>
      </Pressable>
    </View>
  );
}

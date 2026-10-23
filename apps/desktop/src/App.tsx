import { useCallback, useEffect, useState } from "react";
import { api, type DirectoryEntry, type Health, type SignInResponse } from "./api";
import Connecting from "./screens/Connecting";
import Setup from "./screens/Setup";
import Picker from "./screens/Picker";
import SignIn from "./screens/SignIn";
import Shell from "./screens/Shell";

type Stage =
  | { kind: "connecting" }
  | { kind: "setup" }
  | { kind: "picker" }
  | { kind: "sign-in"; account: DirectoryEntry }
  | { kind: "app"; auth: SignInResponse };

/** Lock back to the account picker after this long without input. */
const IDLE_LOCK_MS = 15 * 60 * 1000;

export default function App() {
  const [stage, setStage] = useState<Stage>({ kind: "connecting" });
  const [health, setHealth] = useState<Health | null>(null);

  // Wait for the gateway (it starts alongside the window), then route by setup state.
  useEffect(() => {
    if (stage.kind !== "connecting") return;
    let cancelled = false;
    const poll = async () => {
      while (!cancelled) {
        try {
          const h = await api.health();
          if (cancelled) return;
          setHealth(h);
          setStage(h.setup_required ? { kind: "setup" } : { kind: "picker" });
          return;
        } catch {
          await new Promise((r) => setTimeout(r, 700));
        }
      }
    };
    void poll();
    return () => {
      cancelled = true;
    };
  }, [stage.kind]);

  const lock = useCallback(async (token?: string) => {
    if (token) await api.signOut(token).catch(() => undefined);
    setStage({ kind: "picker" });
  }, []);

  // Idle lock while signed in.
  useEffect(() => {
    if (stage.kind !== "app") return;
    const token = stage.auth.token;
    let timer = window.setTimeout(() => void lock(token), IDLE_LOCK_MS);
    const reset = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(() => void lock(token), IDLE_LOCK_MS);
    };
    const events = ["pointerdown", "keydown", "wheel"] as const;
    events.forEach((e) => window.addEventListener(e, reset, { passive: true }));
    return () => {
      window.clearTimeout(timer);
      events.forEach((e) => window.removeEventListener(e, reset));
    };
  }, [stage, lock]);

  switch (stage.kind) {
    case "connecting":
      return <Connecting />;
    case "setup":
      return <Setup onDone={(auth) => setStage({ kind: "app", auth })} />;
    case "picker":
      return <Picker machineName={health?.name} onPick={(account) => setStage({ kind: "sign-in", account })} />;
    case "sign-in":
      return (
        <SignIn
          account={stage.account}
          onBack={() => setStage({ kind: "picker" })}
          onDone={(auth) => setStage({ kind: "app", auth })}
        />
      );
    case "app":
      return (
        <Shell
          auth={stage.auth}
          version={health?.version ?? ""}
          onLock={() => void lock(stage.auth.token)}
          onSignedOutElsewhere={() => setStage({ kind: "picker" })}
        />
      );
  }
}

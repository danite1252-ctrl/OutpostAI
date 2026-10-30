import { useCallback, useEffect, useState } from "react";
import {
  api,
  ApiError,
  GATEWAY,
  saveServerToken,
  SERVER,
  type DirectoryEntry,
  type Health,
  type ServerProbe,
  type SignInResponse,
} from "./api";
import { ConnectServer, PinPromptHost } from "./components/server";
import Connecting from "./screens/Connecting";
import Setup from "./screens/Setup";
import EngineSetup from "./screens/EngineSetup";
import Picker from "./screens/Picker";
import SignIn from "./screens/SignIn";
import Shell from "./screens/Shell";
import { clearImageCache } from "./components/images";

type Stage =
  | { kind: "connecting" }
  | { kind: "setup" }
  | { kind: "engines"; auth: SignInResponse }
  | { kind: "picker" }
  | { kind: "sign-in"; account: DirectoryEntry }
  | { kind: "app"; auth: SignInResponse }
  // Connecting this app to a server (from the first-run screen or Settings).
  | { kind: "server" }
  // Using a server, but not signed in to it (unpaired, or it was reset).
  | { kind: "server-again"; probe: ServerProbe }
  | { kind: "unreachable"; reason: string };

/** Lock back to the account picker after this long without input. */
const IDLE_LOCK_MS = 15 * 60 * 1000;

export default function App() {
  const [stage, setStage] = useState<Stage>({ kind: "connecting" });
  const [health, setHealth] = useState<Health | null>(null);

  // Wait for the gateway (it starts alongside the window), then route by setup state.
  // Using a server: reach it through the tunnel and carry on with this app's sign-in.
  useEffect(() => {
    if (stage.kind !== "connecting") return;
    let cancelled = false;
    const poll = async () => {
      const started = Date.now();
      while (!cancelled) {
        try {
          const h = await api.health();
          if (cancelled) return;
          setHealth(h);
          if (!SERVER) {
            setStage(h.setup_required ? { kind: "setup" } : { kind: "picker" });
            return;
          }
          const probe: ServerProbe = { host: SERVER.host, port: SERVER.port, fingerprint: SERVER.fingerprint, words: SERVER.words, health: h };
          const token = SERVER.token;
          if (token && !h.setup_required) {
            try {
              const me = await api.me(token);
              if (!cancelled) setStage({ kind: "app", auth: { token, ...me } });
              return;
            } catch (e) {
              if (!(e instanceof ApiError && e.status === 401)) throw e;
              await saveServerToken(null).catch(() => undefined);
            }
          }
          if (!cancelled) setStage({ kind: "server-again", probe });
          return;
        } catch (e) {
          if (SERVER && Date.now() - started > 8000) {
            if (!cancelled) setStage({ kind: "unreachable", reason: e instanceof ApiError ? e.message : "No answer." });
            return;
          }
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
    // The next person to sign in shouldn't inherit this account's images.
    clearImageCache();
    setStage({ kind: "picker" });
  }, []);

  // Idle lock while signed in. (Using a server, this app is a paired device that stays
  // signed in, like a phone.)
  useEffect(() => {
    if (stage.kind !== "app" || SERVER) return;
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

  // Signed out by the server (unpaired elsewhere): pair again.
  const signedOut = useCallback(() => {
    if (SERVER) {
      void saveServerToken(null).finally(() => setStage({ kind: "connecting" }));
    } else {
      setStage({ kind: "picker" });
    }
  }, []);

  return (
    <>
      <PinPromptHost />
      {screen()}
    </>
  );

  function screen() {
  switch (stage.kind) {
    case "connecting":
      return <Connecting server={SERVER?.name} />;
    case "unreachable":
      return <Connecting server={SERVER?.name} problem={stage.reason} onRetry={() => setStage({ kind: "connecting" })} />;
    case "server":
      return <ConnectServer onCancel={() => setStage({ kind: "connecting" })} />;
    case "server-again":
      return <ConnectServer again={{ probe: stage.probe, base: GATEWAY }} />;
    case "setup":
      return (
        <Setup
          onDone={(auth) => setStage({ kind: "engines", auth })}
          onUseServer={() => setStage({ kind: "server" })}
        />
      );
    case "engines":
      return <EngineSetup auth={stage.auth} onDone={() => setStage({ kind: "app", auth: stage.auth })} />;
    case "picker":
      return (
        <Picker
          machineName={health?.name}
          onPick={(account) => setStage({ kind: "sign-in", account })}
          onUseServer={() => setStage({ kind: "server" })}
        />
      );
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
          onLock={SERVER ? undefined : () => void lock(stage.auth.token)}
          onSignedOutElsewhere={signedOut}
          onUseServer={() => setStage({ kind: "server" })}
        />
      );
  }
  }
}

import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type NetworkStatus } from "../api";
import { Button, ErrorNote, StatusDot } from "./ui";

/** Owner switch: let other machines on the network (e.g. Hermes) use app keys. */
export function NetworkPanel({ token }: { token: string }) {
  const [status, setStatus] = useState<NetworkStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const fail = (err: unknown) => setError(err instanceof ApiError ? err.message : "Something went wrong.");
  const load = useCallback(() => {
    api.network(token).then(setStatus, fail);
  }, [token]);
  useEffect(load, [load]);

  async function toggle() {
    if (!status) return;
    const turnOn = !status.lan_access;
    if (
      turnOn &&
      !window.confirm(
        "Other devices on your network will be able to reach Outpost with an app key. PIN sign-in stays on this PC. " +
          "Only do this on a network you trust (home Wi-Fi, not public Wi-Fi). Continue?",
      )
    )
      return;
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.setNetwork(token, turnOn));
    } catch (err) {
      fail(err);
    } finally {
      setBusy(false);
    }
  }

  const open = status?.listening_on ? !status.listening_on.startsWith("127.") : false;

  return (
    <div className="flex flex-col gap-4">
      <div>
        <h2 className="text-lg font-semibold">Network</h2>
        <p className="text-sm text-muted">
          Lets apps on other machines, like Hermes on another PC, connect with an app key. Signing in with a PIN, the
          account list and first-run setup always stay on this PC. Ollama and ComfyUI are never exposed.
        </p>
      </div>
      <div className="flex flex-wrap items-center gap-4 rounded-2xl border border-line bg-surface p-5">
        <span className="flex flex-1 flex-col gap-1">
          <span className="flex items-center gap-2 font-medium">
            <StatusDot on={open} />
            Allow other devices on my network
          </span>
          <span className="text-xs text-muted">
            {status?.listening_on ? `Listening on ${status.listening_on}` : "Checking…"}
            {status?.lan_address ? ` · this PC is ${status.lan_address}` : ""}
          </span>
        </span>
        <Button variant={status?.lan_access ? "ghost" : "primary"} disabled={!status || busy} onClick={() => void toggle()}>
          {status?.lan_access ? "Turn off" : "Turn on"}
        </Button>
      </div>
      {status?.restart_required && (
        <p className="rounded-2xl border border-ember bg-surface p-4 text-sm" role="status">
          Saved. Close and reopen Outpost to {status.lan_access ? "start" : "stop"} listening on the network.
          {status.lan_access && " Windows may ask whether to allow Outpost through the firewall: choose Private networks."}
        </p>
      )}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

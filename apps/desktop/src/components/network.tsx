import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type NetworkStatus } from "../api";
import { ErrorNote, StatusDot, Toggle } from "./ui";

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
        "Other devices on your network will be able to reach Outpost with an app key or a paired phone. PIN sign-in stays on this PC. " +
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

  async function toggleTailscale() {
    if (!status) return;
    const turnOn = !status.tailscale_access;
    if (
      turnOn &&
      !window.confirm(
        "Your devices on Tailscale (your phone away from home, Hermes on another machine) will be able to reach Outpost with an app key or a paired phone. Nothing is opened to the internet. Continue?",
      )
    )
      return;
    setBusy(true);
    setError(null);
    try {
      setStatus(await api.setNetwork(token, status.lan_access, turnOn));
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
          Lets apps on other machines, like Hermes on another PC, connect with an app key (port 7420), and paired phones
          connect over the encrypted port (7421). Signing in with a PIN, the account list and first-run setup stay on
          this PC. Ollama and ComfyUI are never exposed.
        </p>
      </div>
      <div className="flex items-center gap-4 rounded-2xl border border-line bg-surface p-5">
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
        <Toggle
          label="Allow other devices on my network"
          checked={!!status?.lan_access}
          disabled={!status || busy}
          onChange={() => void toggle()}
        />
      </div>
      <div className="flex items-center gap-4 rounded-2xl border border-line bg-surface p-5">
        <span className="flex flex-1 flex-col gap-1">
          <span className="flex items-center gap-2 font-medium">
            <StatusDot on={!!status?.tailscale_running} />
            Allow my Tailscale devices
          </span>
          <span className="text-xs text-muted">
            {!status
              ? "Checking…"
              : status.tailscale
                ? `This PC on Tailscale: ${status.tailscale.dns_name ?? status.tailscale.ip}${
                    status.tailscale.dns_name ? ` (${status.tailscale.ip})` : ""
                  }. Your phone uses it away from home.`
                : "Tailscale isn't running on this PC. Install it and sign in, then reopen this page."}
          </span>
        </span>
        <Toggle
          label="Allow my Tailscale devices"
          checked={!!status?.tailscale_access}
          disabled={!status || busy || (!status.tailscale && !status.tailscale_access)}
          onChange={() => void toggleTailscale()}
        />
      </div>
      {status && status.phone_hosts.length > 0 && (
        <p className="text-xs text-muted">
          Phones try, in order: <span className="font-mono">{status.phone_hosts.join(" → ")}</span>
        </p>
      )}
      {status?.restart_required && (
        <p className="rounded-2xl border border-ember bg-surface p-4 text-sm" role="status">
          Saved. Close and reopen Outpost to apply the change.
          {(status.lan_access || status.tailscale_access) &&
            " Windows may ask whether to allow Outpost through the firewall (for both ports): choose Private networks."}
        </p>
      )}
      <ErrorNote>{error}</ErrorNote>
    </div>
  );
}

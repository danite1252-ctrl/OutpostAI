import { useCallback, useEffect, useState } from "react";
import { api, ApiError, type DeviceView, type NetworkStatus, type PairingStatus, type PairingView, type SignInResponse } from "../api";
import { ago } from "../components/context";
import { AppKeysPanel } from "../components/keys";
import { NetworkPanel } from "../components/network";
import { WebhooksPanel } from "../components/webhooks";
import { Button, ErrorNote, Icon, icons } from "../components/ui";

/** Phones, app keys and network access: everything that reaches Outpost from elsewhere. */
export default function Connections({ auth }: { auth: SignInResponse }) {
  const isOwner = auth.account.role === "owner";
  const [network, setNetwork] = useState<NetworkStatus | null>(null);
  const [devices, setDevices] = useState<DeviceView[]>([]);
  const [everyone, setEveryone] = useState(false);
  const [pairing, setPairing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fail = (err: unknown) => setError(err instanceof ApiError ? err.message : "Something went wrong.");
  const loadDevices = useCallback(() => {
    api.devices(auth.token, everyone).then(setDevices, fail);
  }, [auth.token, everyone]);
  useEffect(loadDevices, [loadDevices]);
  useEffect(() => {
    api.network(auth.token).then(setNetwork, () => undefined);
  }, [auth.token]);

  async function revoke(d: DeviceView) {
    const whose = d.account_id === auth.account.id ? "" : ` (${d.account_name}'s)`;
    if (!window.confirm(`Unpair "${d.label}"${whose}? It is signed out at once and must pair again.`)) return;
    try {
      await api.revokeDevice(auth.token, d.id);
      loadDevices();
    } catch (err) {
      fail(err);
    }
  }

  const ready = network?.secure_port != null;

  return (
    <section className="mx-auto flex max-w-4xl flex-col gap-10">
      <div>
        <h1 className="text-2xl font-semibold">Connections</h1>
        <p className="text-sm text-muted">Phones, apps like Hermes, and who can reach this Outpost from the network.</p>
      </div>

      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-end gap-3">
          <div className="flex-1">
            <h2 className="text-lg font-semibold">Phones</h2>
            <p className="text-sm text-muted">
              A paired phone stays signed in to its account over Outpost's encrypted port. It asks for the PIN again
              before risky things like deleting models or making app keys.
            </p>
          </div>
          <Button onClick={() => setPairing(true)} disabled={!ready}>
            Pair a phone
          </Button>
        </div>
        {!ready && network && (
          <p className="rounded-2xl border border-line bg-surface p-4 text-sm" role="status">
            {network.lan_access || network.tailscale_access
              ? "Network access is on but not running yet: close and reopen Outpost, then pair."
              : isOwner
                ? "Turn on \"Allow other devices on my network\" or \"Allow my Tailscale devices\" below, then restart Outpost to pair a phone."
                : "Ask the owner to turn on network access so phones can pair."}
          </p>
        )}
        {ready && network && (
          <p className="text-xs text-muted">
            Encrypted port {network.secure_port}
            {network.phone_hosts.length ? ` on ${network.phone_hosts.join(", ")}` : ""} · check words{" "}
            <span className="font-mono text-soft">{network.words.join(" · ")}</span>
          </p>
        )}
        {isOwner && (
          <label className="flex items-center gap-2 self-start text-sm text-muted">
            <input type="checkbox" checked={everyone} onChange={(e) => setEveryone(e.target.checked)} className="accent-ember" />
            Show everyone's phones
          </label>
        )}
        <ErrorNote>{error}</ErrorNote>
        {devices.length === 0 ? (
          <p className="text-sm text-muted">No phones paired yet.</p>
        ) : (
          <ul className="flex flex-col divide-y divide-line rounded-2xl border border-line bg-surface">
            {devices.map((d) => (
              <li key={d.id} className="flex flex-wrap items-center gap-3 px-4 py-3">
                <span className="flex min-w-0 flex-1 flex-col">
                  <span className="truncate font-medium">{d.label}</span>
                  <span className="text-xs text-muted">
                    {everyone ? `${d.account_name} · ` : ""}paired {new Date(d.created_at * 1000).toLocaleDateString()} · last used{" "}
                    {ago(d.last_used_at)}
                  </span>
                </span>
                <Button variant="danger" onClick={() => void revoke(d)}>
                  Unpair
                </Button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <AppKeysPanel auth={auth} />
      <WebhooksPanel token={auth.token} />
      {isOwner && <NetworkPanel token={auth.token} />}

      {pairing && (
        <PairDialog
          token={auth.token}
          onClose={() => {
            setPairing(false);
            loadDevices();
          }}
        />
      )}
    </section>
  );
}

/** The code, QR and check words; closes itself into a "paired" message once the phone finishes. */
function PairDialog({ token, onClose }: { token: string; onClose: () => void }) {
  const [view, setView] = useState<PairingView | null>(null);
  const [status, setStatus] = useState<PairingStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [now, setNow] = useState(() => Date.now() / 1000);

  const make = useCallback(() => {
    setError(null);
    setStatus(null);
    api.createPairing(token).then(setView, (err: unknown) =>
      setError(err instanceof ApiError ? err.message : "Couldn't make a code."),
    );
  }, [token]);
  useEffect(make, [make]);

  // Watch for the phone finishing, and count down.
  useEffect(() => {
    if (!view || status?.used) return;
    const t = window.setInterval(() => {
      setNow(Date.now() / 1000);
      api.pairingStatus(token, view.id).then(setStatus, () => undefined);
    }, 1500);
    return () => window.clearInterval(t);
  }, [token, view, status?.used]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const left = view ? Math.max(0, Math.round(view.expires_at - now)) : 0;
  const expired = status?.expired || (view != null && left === 0 && !status?.used);

  return (
    <div role="dialog" aria-modal="true" aria-label="Pair a phone" className="fixed inset-0 z-40 flex items-center justify-center bg-black/80 p-6" onClick={onClose}>
      <div className="flex w-full max-w-2xl flex-col gap-5 rounded-2xl border border-line bg-surface p-6" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center">
          <h2 className="flex-1 text-xl font-semibold">Pair a phone</h2>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="flex size-10 items-center justify-center rounded-lg text-muted hover:bg-raised hover:text-ink"
          >
            <Icon d={icons.close} size={18} />
          </button>
        </div>
        <ErrorNote>{error}</ErrorNote>

        {status?.used ? (
          <div className="flex flex-col gap-3" role="status">
            <p className="text-lg">
              <span className="text-ok">✓</span> Paired <b>{status.device}</b>
              {status.account_name ? ` for ${status.account_name}` : ""}.
            </p>
            <p className="text-sm text-muted">It stays signed in. You can unpair it any time from Connections.</p>
            <Button onClick={onClose} className="self-start">
              Done
            </Button>
          </div>
        ) : view ? (
          <div className="flex flex-col gap-5 sm:flex-row">
            <img
              src={`data:image/svg+xml;utf8,${encodeURIComponent(view.qr_svg)}`}
              alt="QR code to scan with the Outpost phone app"
              className={`size-64 shrink-0 self-center rounded-xl bg-white p-2 ${expired ? "opacity-20" : ""}`}
            />
            <div className="flex flex-col gap-4">
              <ol className="flex list-decimal flex-col gap-2 pl-5 text-sm">
                <li>Open the Outpost app on the phone, on the same Wi-Fi as this PC.</li>
                <li>Scan this code, or pick this Outpost and type the code below.</li>
                <li>Check the phone shows the same three words.</li>
                <li>Choose your account and enter your PIN on the phone.</li>
              </ol>
              <div>
                <span className="text-xs text-muted">Code</span>
                <p className={`font-mono text-4xl tracking-[0.3em] ${expired ? "text-muted line-through" : ""}`}>
                  {view.code.slice(0, 3)} {view.code.slice(3)}
                </p>
              </div>
              <div>
                <span className="text-xs text-muted">Check words</span>
                <p className="font-mono text-lg text-soft">{view.words.join(" · ")}</p>
              </div>
              <p className="text-xs text-muted">
                {expired ? (
                  <>
                    This code has expired.{" "}
                    <button type="button" onClick={make} className="text-ember hover:text-ember-hover">
                      Make a new one
                    </button>
                  </>
                ) : (
                  `Works once, for ${Math.floor(left / 60)}:${String(left % 60).padStart(2, "0")} more. ${view.name}${
                    view.hosts.length ? ` · ${view.hosts.join(", ")}:${view.port}` : ""
                  }`
                )}
              </p>
            </div>
          </div>
        ) : (
          !error && <p className="text-sm text-muted">Making a code…</p>
        )}
      </div>
    </div>
  );
}

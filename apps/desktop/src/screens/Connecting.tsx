import { useEffect, useState } from "react";
import logo from "../assets/outpost-logo.jpg";
import { serverSetup } from "../api";
import { Button } from "../components/ui";

/** Waiting for the gateway: Outpost starting on this PC, or reaching the server. */
export default function Connecting({
  server,
  problem,
  onRetry,
}: {
  /** The server's name, when this app uses one. */
  server?: string;
  /** Couldn't reach the server. */
  problem?: string;
  onRetry?: () => void;
}) {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const t = window.setTimeout(() => setSlow(true), 10_000);
    return () => window.clearTimeout(t);
  }, []);

  const text = problem
    ? `Can't reach ${server ?? "the server"}. ${problem}`
    : server
      ? `Connecting to ${server}…`
      : slow
        ? "Still starting… if this keeps going, check that nothing else is using port 7420."
        : "Starting Outpost…";

  return (
    <main className="flex h-full flex-col items-center justify-center gap-6 bg-deep px-6 text-center">
      <img src={logo} alt="Outpost" className="glow size-56 rounded-3xl" />
      <p className="max-w-md text-muted" aria-live="polite">
        {text}
      </p>
      {problem && (
        <>
          <p className="max-w-md text-sm text-muted">
            Check that the server is on and Outpost is running there, and that this PC is on the same network
            (or on Tailscale).
          </p>
          <div className="flex gap-3">
            <Button onClick={onRetry}>Try again</Button>
            <Button variant="ghost" onClick={() => void serverSetup.useThisPc()}>
              Use Outpost on this PC instead
            </Button>
          </div>
        </>
      )}
    </main>
  );
}

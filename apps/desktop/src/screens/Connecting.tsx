import { useEffect, useState } from "react";
import logo from "../assets/outpost-logo.jpg";

export default function Connecting() {
  const [slow, setSlow] = useState(false);
  useEffect(() => {
    const t = window.setTimeout(() => setSlow(true), 10_000);
    return () => window.clearTimeout(t);
  }, []);

  return (
    <main className="flex h-full flex-col items-center justify-center gap-6 bg-deep px-6 text-center">
      <img src={logo} alt="Outpost" className="glow size-56 rounded-3xl" />
      <p className="text-muted" aria-live="polite">
        {slow ? "Still starting… if this keeps going, check that nothing else is using port 7420." : "Starting Outpost…"}
      </p>
    </main>
  );
}

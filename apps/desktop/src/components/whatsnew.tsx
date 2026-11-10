// "What's new": after Outpost updates, the notes of each release since the last one this PC
// showed, once. Nothing on a first install, and nothing if GitHub can't be reached.

import { useEffect, useState } from "react";
import { request } from "../api";
import { Button } from "./ui";

const SEEN_KEY = "outpost.whatsnew.seen";

interface ReleaseNotes {
  version: string;
  notes: string;
  date: string | null;
}

function seen(): string | null {
  try {
    return localStorage.getItem(SEEN_KEY);
  } catch {
    return null;
  }
}

function setSeen(v: string) {
  try {
    localStorage.setItem(SEEN_KEY, v);
  } catch {
    // Not remembered: it may show again next start.
  }
}

/** `a` is newer than `b` (1.5.10 > 1.5.9). */
function newer(a: string, b: string) {
  const p = (v: string) => v.replace(/^v/, "").split("-")[0].split(".").map((n) => parseInt(n, 10) || 0);
  const x = p(a);
  const y = p(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) if ((x[i] ?? 0) !== (y[i] ?? 0)) return (x[i] ?? 0) > (y[i] ?? 0);
  return false;
}

async function appVersion(fallback: string): Promise<string> {
  try {
    const { getVersion } = await import("@tauri-apps/api/app");
    return await getVersion();
  } catch {
    return fallback;
  }
}

export function WhatsNew({ token, version }: { token: string; version: string }) {
  const [shown, setShown] = useState<{ version: string; notes: ReleaseNotes[] } | null>(null);

  useEffect(() => {
    let gone = false;
    void (async () => {
      const now = await appVersion(version);
      if (!now) return;
      const last = seen();
      // A first install has nothing to compare with.
      if (!last) return setSeen(now);
      if (!newer(now, last)) return;
      try {
        const notes = await request<ReleaseNotes[]>(
          "GET",
          `/api/updates/notes?since=${encodeURIComponent(last)}&upto=${encodeURIComponent(now)}`,
          { token },
        );
        if (gone) return;
        if (notes.length > 0) setShown({ version: now, notes });
        else setSeen(now);
      } catch {
        // GitHub didn't answer: try again next start.
      }
    })();
    return () => {
      gone = true;
    };
  }, [token, version]);

  if (!shown) return null;
  const close = () => {
    setSeen(shown.version);
    setShown(null);
  };
  return (
    <div role="dialog" aria-modal="true" aria-label="What's new" className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6" onClick={close}>
      <div className="flex max-h-[80vh] w-full max-w-lg flex-col gap-4 rounded-2xl border border-line bg-surface p-6" onClick={(e) => e.stopPropagation()}>
        <h2 className="text-xl font-semibold">What's new in Outpost {shown.version}</h2>
        <ul className="flex min-h-0 flex-col gap-4 overflow-auto">
          {shown.notes.map((n) => (
            <li key={n.version} className="flex flex-col gap-1">
              {shown.notes.length > 1 && <span className="text-xs text-muted">{n.version}</span>}
              <p className="whitespace-pre-wrap text-sm text-soft [overflow-wrap:anywhere]">{n.notes}</p>
            </li>
          ))}
        </ul>
        <Button onClick={close} autoFocus>
          Got it
        </Button>
      </div>
    </div>
  );
}

// "It's ready" notifications: when a long job (a 3D model, a video, a training, or anything
// that took a minute or more) finishes or fails, Windows shows a notification, unless you're
// looking at Outpost already. Checked every 20 seconds while the app is open (also when it's
// hidden in the tray).

import { useEffect, useState } from "react";
import { request } from "../api";
import { ToggleRow } from "./ui";

interface FinishedJob {
  id: string;
  kind: "picture" | "video" | "model" | "training";
  status: "done" | "failed";
  title: string;
  body: string;
  long: boolean;
  finished_at: number;
}

const KEY = "outpost.notify";

export function notificationsOn(): boolean {
  try {
    return localStorage.getItem(KEY) !== "off";
  } catch {
    return true;
  }
}

function setNotificationsOn(on: boolean) {
  try {
    localStorage.setItem(KEY, on ? "on" : "off");
  } catch {
    /* kept for this session only */
  }
}

async function plugin() {
  return import("@tauri-apps/plugin-notification");
}

/** Ask Windows once; true when notifications may be shown. */
async function allowed(): Promise<boolean> {
  try {
    const n = await plugin();
    if (await n.isPermissionGranted()) return true;
    return (await n.requestPermission()) === "granted";
  } catch {
    return false;
  }
}

/** Watch for finished jobs and notify (mounted once, in the app's shell). */
export function useJobNotifications(token: string) {
  useEffect(() => {
    let live = true;
    let since = Math.floor(Date.now() / 1000);
    const seen = new Set<string>();
    const check = async () => {
      if (!notificationsOn()) return;
      try {
        const r = await request<{ jobs: FinishedJob[]; now: number }>("GET", `/api/jobs/finished?since=${since}`, { token });
        if (!live) return;
        for (const j of r.jobs) {
          if (seen.has(j.id)) continue;
          seen.add(j.id);
          // Looking at Outpost already: the queue shows it.
          const watching = document.visibilityState === "visible" && document.hasFocus();
          if (j.long && !watching && (await allowed())) {
            (await plugin()).sendNotification({ title: j.title, body: j.body || undefined });
          }
        }
        // Go back a little: a job finishing during this look is caught next time (seen skips repeats).
        since = Math.max(since, r.now - 5);
        if (seen.size > 200) seen.clear();
      } catch {
        /* offline for a moment: try again next time */
      }
    };
    const t = window.setInterval(() => void check(), 20_000);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [token]);
}

/** Settings: the switch. */
export function NotificationsPanel() {
  const [on, setOn] = useState(notificationsOn);
  return (
    <ToggleRow
      title="Tell me when long jobs finish"
      hint="A Windows notification when a 3D model, video or training (or anything that took a minute or more) is ready or fails, unless Outpost is in front."
      checked={on}
      onChange={(v) => {
        setOn(v);
        setNotificationsOn(v);
        if (v) void allowed();
      }}
    />
  );
}

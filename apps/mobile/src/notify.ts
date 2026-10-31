/** "It's ready" notifications: Android checks the Outpost in the background (see the native
 * JobNotifier) and shows one when a long job (3D, video, training...) finishes or fails. */
import * as SecureStore from "expo-secure-store";
import OutpostNet from "../modules/outpost-net";
import type { Outpost } from "./outpost";
import { loadNotify, saveNotify, type Saved } from "./store";

const ASKED = "outpost.notify.asked.v1";

/** Start (or stop) background checks for this pairing, as the setting says. */
export async function setUpNotifications(saved: Saved | null, ask: boolean): Promise<void> {
  try {
    const on = saved != null && (await loadNotify());
    // Asked once; after that it's the switch in Me, or Android's settings.
    if (on && ask && (await SecureStore.getItemAsync(ASKED).catch(() => null)) !== "1") {
      await SecureStore.setItemAsync(ASKED, "1").catch(() => undefined);
      await OutpostNet.askNotifications();
    }
    OutpostNet.notifyConfigure(on, saved?.hosts ?? [], saved?.port ?? 0, saved?.fingerprint ?? "", saved?.token ?? "");
  } catch {
    // An older app build without notifications: nothing to do.
  }
}

export async function setNotify(saved: Saved, on: boolean): Promise<boolean> {
  await saveNotify(on);
  let allowed = true;
  if (on) {
    try {
      allowed = await OutpostNet.askNotifications();
    } catch {
      allowed = false;
    }
  }
  await setUpNotifications(saved, false);
  return allowed;
}

export function inFront(front: boolean) {
  try {
    OutpostNet.notifyInFront(front);
  } catch {
    // Older build.
  }
}

/** While the app is open: if jobs of mine are waiting or running, start watching for them
 * (so the notification comes the moment one finishes, even after the app is left). */
export async function watchIfRunning(api: Outpost): Promise<void> {
  try {
    if (!(await loadNotify())) return;
    const r = await api.get<{ unfinished: number }>(`/api/jobs/finished?since=${Math.floor(Date.now() / 1000) + 60}`);
    if (r.unfinished > 0) OutpostNet.watchJobs();
  } catch {
    // Offline or an older build: the regular checks cover it.
  }
}

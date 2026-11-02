// Tools on this PC for chats from the phone: while the app is open (even in the tray), it
// tells Outpost which tool servers it runs and which chats and personas use them, and runs
// the tool calls those chats' replies make.

import { useEffect } from "react";
import { GATEWAY, pcTools, type MessageTools } from "../api";
import { toolsAvailable } from "./tools";

const EVERY_MS = 20_000;

async function pcName(): Promise<string> {
  try {
    const { invoke } = await import("@tauri-apps/api/core");
    return await invoke<string>("pc_name");
  } catch {
    return "this PC";
  }
}

async function post(token: string, path: string, body?: unknown, signal?: AbortSignal) {
  return fetch(`${GATEWAY}${path}`, {
    method: "POST",
    headers: { Authorization: `Bearer ${token}`, "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });
}

/** Offer this PC's tools for your chats from elsewhere, while signed in. */
export function useDeviceTools(token: string) {
  useEffect(() => {
    if (!toolsAvailable()) return;
    let alive = true;
    const stop = new AbortController();
    let offered = false;

    async function register() {
      try {
        const [servers, sel, name] = await Promise.all([pcTools.list(), pcTools.allSelections(), pcName()]);
        // Only servers some chat or persona uses get started and offered.
        const used = new Set([...Object.values(sel.chats).flat(), ...Object.values(sel.personas).flat()]);
        const ids = servers.filter((s) => s.enabled && used.has(s.id)).map((s) => s.id);
        const tools: MessageTools[] = ids.length ? await pcTools.messageTools(ids) : [];
        const offer = tools
          .filter((s) => s.tools.length)
          .map((s) => ({
            id: s.id,
            name: s.name,
            tools: s.tools.map((t) => ({ name: t.name, description: t.description, inputSchema: t.inputSchema, read_only: t.read_only })),
          }));
        const r = await post(token, "/api/pc-tools/register", { name, servers: offer, chats: sel.chats, personas: sel.personas }, stop.signal);
        offered = r.ok && offer.length > 0;
      } catch {
        offered = false;
      }
    }

    async function work() {
      while (alive) {
        if (!offered) {
          await new Promise((r) => setTimeout(r, 5000));
          continue;
        }
        try {
          const r = await post(token, "/api/pc-tools/next", undefined, stop.signal);
          if (r.status === 200) {
            const req = (await r.json()) as { id: string; server: string; tool: string; arguments: unknown };
            void pcTools
              .call(req.server, req.tool, req.arguments ?? {})
              .catch((e) => ({ error: typeof e === "string" ? e : "The tool couldn't run on this PC." }))
              .then((answer) => post(token, `/api/pc-tools/answers/${req.id}`, answer).catch(() => undefined));
          } else if (r.status !== 204) {
            // An older Outpost, or signed out: try again later.
            await new Promise((res) => setTimeout(res, 15_000));
          }
        } catch {
          if (!alive) return;
          await new Promise((res) => setTimeout(res, 5000));
        }
      }
    }

    void register();
    const timer = window.setInterval(() => void register(), EVERY_MS);
    void work();
    return () => {
      alive = false;
      stop.abort();
      window.clearInterval(timer);
    };
  }, [token]);
}

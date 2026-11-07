// The activity log on the phone: every tool call a persona made, newest first. Owners see
// everyone's; people see their own.

import { useCallback, useEffect, useState } from "react";
import { Text, View } from "react-native";
import { ListState, message, Screen } from "./admin";
import { type Outpost } from "./outpost";
import { Button, C, s } from "./ui";

interface Entry {
  id: string;
  at: number;
  account_id: string;
  person: string | null;
  chat_title: string | null;
  routine_name: string | null;
  source: "chat" | "hands_free" | "routine";
  server: string;
  tool: string;
  arguments: string;
  outcome: "done" | "failed" | "not_allowed" | "skipped" | "needs_ok";
  result: string;
}

const OUTCOME: Record<Entry["outcome"], { label: string; color: string }> = {
  done: { label: "Done", color: C.ok },
  failed: { label: "Failed", color: C.danger },
  not_allowed: { label: "Not allowed", color: C.danger },
  skipped: { label: "Skipped", color: C.amber },
  needs_ok: { label: "Needed an OK", color: C.amber },
};

function when(at: number): string {
  const d = new Date(at * 1000);
  const time = d.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
  if (d.toDateString() === new Date().toDateString()) return time;
  return `${d.toLocaleDateString(undefined, { day: "numeric", month: "short" })} ${time}`;
}

function argsWords(json: string): string {
  try {
    const v = JSON.parse(json) as Record<string, unknown>;
    if (!v || typeof v !== "object" || Array.isArray(v)) return json;
    const parts = Object.entries(v).map(([k, x]) => `${k}: ${Array.isArray(x) ? x.join(", ") : typeof x === "object" && x ? JSON.stringify(x) : String(x)}`);
    return parts.length ? parts.join(" · ") : "(nothing)";
  } catch {
    return json;
  }
}

export function ActivityScreen({ api, isOwner, onClose }: { api: Outpost; isOwner: boolean; onClose: () => void }) {
  const [entries, setEntries] = useState<Entry[] | null>(null);
  const [more, setMore] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [open, setOpen] = useState<string | null>(null);

  const load = useCallback(
    async (before?: number) => {
      setError(null);
      try {
        const page = await api.get<Entry[]>(`/api/activity?limit=50${before ? `&before=${before}` : ""}`);
        setEntries((list) => (before ? [...(list ?? []), ...page] : page));
        setMore(page.length === 50);
      } catch (e) {
        setError(message(e));
      }
    },
    [api],
  );
  useEffect(() => {
    void load();
  }, [load]);

  return (
    <Screen title="Activity" onClose={onClose}>
      <Text style={{ color: C.muted }}>
        Every tool call a persona made{isOwner ? ", for everyone" : " for you"}, exactly as the model sent it. Kept for 90 days.
      </Text>
      <ListState
        loading={entries === null}
        error={error}
        empty={entries?.length === 0}
        emptyText="No tool calls yet. When a persona uses a tool (Home Assistant, Google, search…), it shows here."
      />
      {(entries ?? []).map((e) => {
        const o = OUTCOME[e.outcome] ?? OUTCOME.done;
        const from = e.source === "routine" ? `Routine "${e.routine_name ?? "deleted"}"` : e.source === "hands_free" ? "Hands-free" : (e.chat_title ?? "A chat");
        return (
          <View key={e.id} style={[s.card, { gap: 4 }]}>
            <Text style={{ color: C.muted, fontSize: 12 }}>
              {when(e.at)}
              {isOwner ? ` · ${e.person ?? "Someone"}` : ""} · {from}
            </Text>
            <Text style={{ color: C.ink }}>
              {e.server} › {e.tool} <Text style={{ color: o.color }}>· {o.label}</Text>
            </Text>
            <Text style={{ color: C.soft, fontSize: 12, fontFamily: "monospace" }}>{argsWords(e.arguments)}</Text>
            {open === e.id ? (
              <Text style={{ color: C.soft, fontSize: 12 }} onPress={() => setOpen(null)}>
                {e.result || "(nothing came back)"}
              </Text>
            ) : (
              <Text style={{ color: C.muted, fontSize: 12 }} onPress={() => setOpen(e.id)}>
                What came back ›
              </Text>
            )}
          </View>
        );
      })}
      {entries && entries.length > 0 && more && <Button kind="ghost" title="Older" onPress={() => void load(entries[entries.length - 1].at)} />}
      <Button kind="ghost" title="Refresh" onPress={() => void load()} />
    </Screen>
  );
}

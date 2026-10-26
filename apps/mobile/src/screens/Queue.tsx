import { useCallback, useEffect, useState } from "react";
import { Pressable, ScrollView, Text, View } from "react-native";
import { eta, fraction, type Job, type LineEntry, type Outpost } from "../outpost";
import { Avatar, Bar, C, ErrorText, H1, P, s } from "../ui";

/** The shared line for the GPU, polled quickly while anything is in it. */
export function useLine(api: Outpost, focused: boolean) {
  const [line, setLine] = useState<LineEntry[]>([]);
  const busy = line.length > 0;
  useEffect(() => {
    let live = true;
    const tick = () =>
      api.get<LineEntry[]>("/api/queue").then(
        (l) => live && setLine(l),
        () => undefined,
      );
    void tick();
    const t = setInterval(tick, busy || focused ? 2000 : 15000);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [api, busy, focused]);
  return line;
}

function promptOf(j: Job | undefined) {
  try {
    return j ? ((JSON.parse(j.inputs).prompt as string) ?? "") : "";
  } catch {
    return "";
  }
}

export default function Queue({ api, line, onError }: { api: Outpost; line: LineEntry[]; onError: (e: unknown) => void }) {
  const [jobs, setJobs] = useState<Job[]>([]);
  const [error, setError] = useState<string | null>(null);
  const load = useCallback(
    () =>
      api.get<Job[]>("/api/jobs?limit=20").then(setJobs, (e) => {
        onError(e);
        setError(e instanceof Error ? e.message : "Couldn't load your jobs.");
      }),
    [api, onError],
  );
  useEffect(() => {
    void load();
  }, [load, line.length]);

  async function cancel(id: string) {
    try {
      await api.json("DELETE", `/api/jobs/${id}`);
      void load();
    } catch (e) {
      onError(e);
    }
  }

  const now = Date.now() / 1000;
  const recent = jobs.filter((j) => (j.status === "done" || j.status === "failed") && (j.finished_at ?? 0) > now - 3600).slice(0, 6);
  const runningAt = line.findIndex((e) => e.status === "running");

  return (
    <ScrollView style={s.screen} contentContainerStyle={s.pad}>
      <H1>Queue</H1>
      <P muted>One job runs at a time on the graphics card, in the order they were added.</P>
      <ErrorText>{error}</ErrorText>
      {line.length === 0 && <P muted>Nothing waiting.</P>}
      {line.map((e, i) => {
        const running = e.status === "running";
        const f = fraction(e.progress);
        const job = jobs.find((j) => j.id === e.job_id);
        const title = e.mine ? promptOf(job) || (e.pack_id === "train" ? "Training a character" : "Your job") : running ? `${e.by_name} is creating` : `${e.by_name}'s job`;
        const ahead = i - (runningAt >= 0 ? 1 : 0);
        const detail = running
          ? [
              e.progress?.phase,
              e.progress && e.progress.steps > 0 ? `step ${e.progress.step} of ${e.progress.steps}` : null,
              f != null ? `${Math.round(f * 100)}%` : null,
              e.eta_secs != null ? eta(e.eta_secs) : null,
            ]
              .filter(Boolean)
              .join(" · ")
          : ahead === 0
            ? "Next up"
            : `${ahead} ahead`;
        return (
          <View key={e.job_id ?? `${e.by_name}-${i}`} style={s.card}>
            <View style={s.row}>
              <Avatar name={e.by_name} color={e.by_color} size={28} />
              <View style={{ flex: 1 }}>
                <Text numberOfLines={2} style={{ color: C.ink, fontWeight: e.mine ? "600" : "400" }}>
                  {title}
                </Text>
                <Text style={{ color: C.muted, fontSize: 13 }}>{detail}</Text>
              </View>
              {e.mine && e.job_id && (
                <Pressable accessibilityRole="button" onPress={() => void cancel(e.job_id!)} style={[s.chip, { minHeight: 36 }]}>
                  <Text style={{ color: C.ink }}>{running ? "Stop" : "Cancel"}</Text>
                </Pressable>
              )}
            </View>
            {running && <Bar fraction={f} color={e.mine ? C.ember : C.pick} />}
          </View>
        );
      })}
      {recent.length > 0 && <Text style={[s.label, { marginTop: 8 }]}>Finished in the last hour</Text>}
      {recent.map((j) => (
        <View key={j.id} style={s.card}>
          <Text numberOfLines={2} style={{ color: C.ink }}>
            {promptOf(j) || (j.pack_id === "train" ? "Training a character" : j.pack_id)}
          </Text>
          <Text style={{ color: j.status === "failed" ? C.danger : C.ok, fontSize: 13 }}>
            {j.status === "failed" ? (j.error ?? "Failed") : "Done · in your gallery"}
          </Text>
        </View>
      ))}
    </ScrollView>
  );
}

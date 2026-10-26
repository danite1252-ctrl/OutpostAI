import { useCallback, useEffect, useRef, useState } from "react";
import { BackHandler, Modal, ScrollView, Text, View } from "react-native";
import OutpostNet, { type ConverseEvent } from "../../modules/outpost-net";
import type { Message, Outpost, Persona } from "../outpost";
import { Avatar, Button, C, Chip, s } from "../ui";
import { allowMicrophone, say, stopSaying } from "../voice";
import { VoiceWave, type WaveMode } from "../wave";

type Phase = "starting" | "listening" | "hearing" | "thinking" | "speaking" | "paused";

/** Remembered while the app is open: whether talking over a reply stops it. */
let talkOver = true;

/** Whisper's usual guesses for a breath or background noise. */
const NOT_SPEECH = /^\s*(you|thank you for watching|thanks for watching|\.+|…)[.!]?\s*$/i;

/**
 * A spoken conversation, like a call: the microphone stays open (with the phone's echo
 * cancelling), it notices when you start and stop talking (no beeps), your PC's Whisper
 * works out the words, and the reply is read aloud. Talking over a reply stops it.
 */
export default function HandsFree({
  api,
  chatId,
  persona,
  onTurn,
  onClose,
}: {
  api: Outpost;
  chatId: string;
  persona: Persona | null;
  /** A turn finished: reload the chat. */
  onTurn: () => void;
  onClose: () => void;
}) {
  const [phase, setPhase] = useState<Phase>("starting");
  const [heard, setHeard] = useState("");
  const [reply, setReply] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [level, setLevel] = useState(0);
  const [over, setOver] = useState(talkOver);
  const [talking, setTalking] = useState(false);
  const [voiceLevel, setVoiceLevel] = useState(0);
  /** Just finished hearing something: shows "Got it" for a moment. */
  const [gotIt, setGotIt] = useState(false);
  const live = useRef(true);
  const phaseRef = useRef<Phase>("starting");
  const queue = useRef<string[]>([]);
  const busy = useRef(false);
  const stopStream = useRef<(() => void) | null>(null);

  const go = useCallback((p: Phase) => {
    phaseRef.current = p;
    setPhase(p);
  }, []);

  const end = useCallback(() => {
    live.current = false;
    stopStream.current?.();
    stopSaying();
    OutpostNet.stopConversation();
    onClose();
  }, [onClose]);

  useEffect(() => {
    const sub = BackHandler.addEventListener("hardwareBackPress", () => {
      end();
      return true;
    });
    return () => sub.remove();
  }, [end]);

  /** Send one thing said; resolves with the reply's text. */
  const ask = useCallback(
    (text: string) =>
      new Promise<string>((resolve) => {
        let said = "";
        stopStream.current = api.stream(`/api/chats/${chatId}/messages`, { content: text, images: [] }, {
          event: (name, data) => {
            const d = data as { text?: string; message?: Message | string; error?: string | null };
            if (name === "token") {
              said += d.text ?? "";
              setReply(said);
            } else if (name === "done" && d.message && typeof d.message === "object") {
              said = d.message.content;
              if (d.error) setError(d.error);
            } else if (name === "error") setError(String(d.message ?? "The reply failed."));
          },
          done: () => {
            stopStream.current = null;
            resolve(said);
          },
          error: (e) => {
            stopStream.current = null;
            setError(e.message);
            resolve("");
          },
        });
      }),
    [api, chatId],
  );

  /** Work through what was said, one turn at a time. */
  const work = useCallback(async () => {
    if (busy.current) return;
    busy.current = true;
    try {
      while (live.current && queue.current.length) {
        const audio = queue.current.shift()!;
        go("hearing");
        let text = "";
        try {
          text = (await api.transcribe(audio, "audio/wav")).text.trim();
        } catch (e) {
          setError(e instanceof Error ? e.message : "Your PC couldn't work out what you said.");
          continue;
        }
        if (!live.current) return;
        if (!text || NOT_SPEECH.test(text)) continue;
        // Said more while this was being worked out: send it all together.
        while (queue.current.length && phaseRef.current !== "paused") {
          try {
            const more = (await api.transcribe(queue.current.shift()!, "audio/wav")).text.trim();
            if (more && !NOT_SPEECH.test(more)) text = `${text} ${more}`;
          } catch {
            // Keep what we have.
          }
        }
        setHeard(text);
        setReply("");
        setError(null);
        go("thinking");
        const said = await ask(text);
        onTurn();
        if (!live.current) return;
        // You spoke again while it was thinking: answer that instead of reading this out.
        if (queue.current.length || !said.trim()) continue;
        go("speaking");
        try {
          await say(api, said, persona?.id ?? null, "hands-free");
        } catch (e) {
          setError(e instanceof Error ? e.message : "The reply couldn't be read aloud.");
        }
      }
    } finally {
      busy.current = false;
      if (live.current && phaseRef.current !== "paused") go("listening");
    }
  }, [api, ask, go, onTurn, persona]);

  useEffect(() => {
    live.current = true;
    const sub = OutpostNet.addListener("onConverse", (e: ConverseEvent) => {
      if (!live.current) return;
      if (e.type === "ready") go("listening");
      else if (e.type === "level") setLevel(e.level);
      else if (e.type === "start") {
        setTalking(true);
        // The phone already stopped the reply; let the reading-aloud know.
        if (e.interrupted) stopSaying();
      } else if (e.type === "discard") setTalking(false);
      else if (e.type === "utterance") {
        setTalking(false);
        setGotIt(true);
        setTimeout(() => setGotIt(false), 700);
        if (phaseRef.current === "paused") return;
        queue.current.push(e.audio);
        void work();
      } else if (e.type === "error") {
        setError(e.message);
        go("paused");
      }
    });
    // How loud the reply is as it plays, for the wave.
    const playback = OutpostNet.addListener("onPlayback", (e) => {
      if (e.type === "level") setVoiceLevel(e.level ?? 0);
    });
    allowMicrophone().then(
      () => live.current && OutpostNet.converse(talkOver),
      (e) => {
        setError(e instanceof Error ? e.message : "The microphone isn't allowed.");
        go("paused");
      },
    );
    return () => {
      live.current = false;
      sub.remove();
      playback.remove();
      stopSaying();
      OutpostNet.stopConversation();
    };
    // Start once.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  function setTalkOver(on: boolean) {
    talkOver = on;
    setOver(on);
    // Reopen the microphone with the new setting.
    if (phaseRef.current !== "paused") OutpostNet.converse(on);
  }

  const who = persona?.name ?? "Outpost";
  const status =
    phase === "starting"
      ? "Starting the microphone…"
      : phase === "paused"
        ? "Paused"
        : gotIt
          ? "Got it"
          : talking
            ? "Listening…"
            : phase === "hearing" || phase === "thinking"
              ? "Thinking…"
              : phase === "speaking"
                ? `${who} is speaking`
                : "Go ahead, I'm listening";
  const mode: WaveMode =
    phase === "paused" || phase === "starting"
      ? "idle"
      : gotIt
        ? "heard"
        : talking
          ? "listening"
          : phase === "hearing" || phase === "thinking"
            ? "thinking"
            : phase === "speaking"
              ? "speaking"
              : "listening";
  // The persona speaks in its own colour; you're white.
  const waveColor = mode === "speaking" ? (persona?.avatar_color ?? C.amber) : "#ffffff";
  const waveLevel = mode === "speaking" ? voiceLevel : talking || phase === "listening" ? level : 0;
  const shown = reply.replace(/\[\[outpost-job:[^\]]+\]\]/g, "").trim();

  return (
    <Modal visible animationType="fade" onRequestClose={end}>
      <View style={[s.screen, { paddingHorizontal: 20, paddingTop: 28, paddingBottom: 24 }]}>
        <View style={[s.row, { justifyContent: "center" }]}>
          <Avatar name={who} color={persona?.avatar_color ?? C.amber} size={26} />
          <Text style={{ color: C.soft, fontSize: 16, fontWeight: "600" }}>{who}</Text>
        </View>

        <View style={{ flex: 1, justifyContent: "center", gap: 18 }}>
          <VoiceWave level={waveLevel} mode={mode} color={waveColor} height={150} />
          <Text
            accessibilityLiveRegion="polite"
            style={{ color: phase === "paused" ? C.muted : C.ink, fontSize: 17, textAlign: "center", fontWeight: "500" }}
          >
            {status}
          </Text>
          {heard ? (
            <Text numberOfLines={2} style={{ color: C.muted, fontSize: 14, textAlign: "center" }}>
              “{heard}”
            </Text>
          ) : null}
          {shown && (phase === "thinking" || phase === "speaking") ? (
            <ScrollView style={{ maxHeight: 160 }}>
              <Text style={{ color: C.soft, fontSize: 15, lineHeight: 22, textAlign: "center" }}>{shown}</Text>
            </ScrollView>
          ) : null}
          {error ? (
            <Text accessibilityRole="alert" style={[s.error, { textAlign: "center" }]}>
              {error}
            </Text>
          ) : null}
        </View>

        <View style={{ gap: 14, alignItems: "center" }}>
          <View style={[s.row, { justifyContent: "center" }]}>
            {phase === "speaking" && <Button kind="ghost" title="Interrupt" onPress={() => stopSaying()} />}
            {phase === "paused" ? (
              <Button
                kind="ghost"
                title="Resume"
                onPress={() => {
                  setError(null);
                  go("listening");
                  OutpostNet.converseMuted(false);
                  // If the microphone never opened (permission, busy), try again.
                  allowMicrophone().then(
                    () => OutpostNet.converse(talkOver),
                    (e) => setError(e instanceof Error ? e.message : "The microphone isn't allowed."),
                  );
                }}
              />
            ) : (
              <Button
                kind="ghost"
                title="Pause"
                onPress={() => {
                  go("paused");
                  OutpostNet.converseMuted(true);
                  queue.current = [];
                  stopSaying();
                }}
              />
            )}
            <Button kind="danger" title="End" onPress={end} />
          </View>
          <Chip label={over ? "Talk over replies: on" : "Talk over replies: off"} active={over} onPress={() => setTalkOver(!over)} />
          <Text style={{ color: C.muted, fontSize: 12, textAlign: "center" }}>
            Just talk, then pause.{over ? " Talk while it answers to cut in." : ""} Your PC works out what you said.
          </Text>
        </View>
      </View>
    </Modal>
  );
}

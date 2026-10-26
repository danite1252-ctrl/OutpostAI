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

/** Real words, not noise Whisper turned into a word or two of nothing. */
function isSpeech(text: string) {
  const t = text.trim();
  return t.replace(/[^A-Za-z0-9\u00C0-\uFFFF]/g, "").length >= 2 && !NOT_SPEECH.test(t);
}

/** The reply's own words, heard back through the microphone (not the user talking over it). */
function isEcho(heard: string, reply: string) {
  const words = (t: string) => t.toLowerCase().match(/[a-z0-9\u00C0-\uFFFF']{2,}/g) ?? [];
  const h = words(heard);
  if (h.length === 0) return false;
  const r = new Set(words(reply));
  return h.filter((w) => r.has(w)).length / h.length >= 0.6;
}

/** Something said: a recording to work out, or words already worked out. */
type Said = { audio: string } | { text: string };

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
  /** What the microphone hears, for the check line at the bottom. */
  const [meter, setMeter] = useState<{ db: number; room: number; voice: boolean; inSpeech: boolean } | null>(null);
  const [lastStep, setLastStep] = useState("");
  const live = useRef(true);
  const phaseRef = useRef<Phase>("starting");
  const queue = useRef<Said[]>([]);
  /** A reply is paused because someone may be talking over it. */
  const held = useRef(false);
  /** The reply being read aloud, to tell its echo from you talking over it. */
  const speaking = useRef("");
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
        const toText = async (item: Said) => ("text" in item ? item.text : (await api.transcribe(item.audio, "audio/wav")).text.trim());
        go("hearing");
        let text = "";
        try {
          text = await toText(queue.current.shift()!);
        } catch (e) {
          setError(e instanceof Error ? e.message : "Your PC couldn't work out what you said.");
          setLastStep("Sending what you said to your PC failed.");
          continue;
        }
        if (!live.current) return;
        if (!isSpeech(text)) {
          setLastStep(text.trim() ? `Your PC heard “${text.trim()}”: not words, so it was ignored.` : "Your PC heard no words in that.");
          continue;
        }
        setLastStep("");
        // Said more while this was being worked out: send it all together.
        while (queue.current.length && phaseRef.current !== "paused") {
          try {
            const more = await toText(queue.current.shift()!);
            if (isSpeech(more)) text = `${text} ${more}`;
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
        // You spoke again while it was thinking: answer that instead of reading this out
        // (if it was really you, not noise).
        if (!said.trim()) continue;
        if (queue.current.length) {
          const pending = await Promise.all(
            queue.current.map((item) => ("text" in item ? item.text : api.transcribe(item.audio, "audio/wav").then((r) => r.text.trim(), () => ""))),
          );
          queue.current = pending.filter(isSpeech).map((text) => ({ text }));
          if (queue.current.length) continue;
        }
        go("speaking");
        try {
          speaking.current = said;
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
      else if (e.type === "level") {
        setLevel(e.level);
        if (e.db != null) setMeter({ db: e.db, room: e.roomDb ?? -90, voice: !!e.voiceLike, inSpeech: !!e.inSpeech });
      }
      else if (e.type === "start") {
        setTalking(true);
        // The phone paused the reply; it carries on unless this turns out to be you talking.
        if (e.interrupted) held.current = true;
      } else if (e.type === "discard") {
        setTalking(false);
        // Just a bump or a cough: carry on.
        if (held.current) {
          held.current = false;
          OutpostNet.resumePlaying();
        }
      } else if (e.type === "utterance") {
        setTalking(false);
        if (phaseRef.current === "paused") return;
        if (held.current || phaseRef.current === "speaking") {
          // Talking over the reply: stop it only for real words, so noise never cuts it off.
          const wasHeld = held.current;
          held.current = false;
          api.transcribe(e.audio, "audio/wav").then(
            (r) => {
              if (!live.current) return;
              if (isSpeech(r.text) && !isEcho(r.text, speaking.current)) {
                setGotIt(true);
                setTimeout(() => setGotIt(false), 700);
                queue.current.push({ text: r.text.trim() });
                stopSaying(); // The reading-aloud ends and the turn loop picks this up.
                void work();
              } else if (wasHeld) OutpostNet.resumePlaying();
            },
            () => {
              if (wasHeld) OutpostNet.resumePlaying();
            },
          );
          return;
        }
        setGotIt(true);
        setTimeout(() => setGotIt(false), 700);
        setLastStep(`Heard ${(((e.audio.length * 3) / 4 - 44) / 32000).toFixed(1)} s, sending it to your PC…`);
        queue.current.push({ audio: e.audio });
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
                  held.current = false;
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
          {meter ? (
            <Text style={{ color: C.muted, fontSize: 11, textAlign: "center", fontVariant: ["tabular-nums"] }}>
              Mic {Math.round(meter.db)} dB · room {Math.round(meter.room)} dB · {meter.inSpeech ? "hearing you" : meter.voice ? "voice-like" : "quiet"}
              {lastStep ? `\n${lastStep}` : ""}
            </Text>
          ) : null}
        </View>
      </View>
    </Modal>
  );
}

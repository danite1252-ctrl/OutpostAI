// Pieces the phone's management screens share: a full screen with a title and Close, the
// "enter your PIN again" prompt that risky actions need from a paired phone, and on/off rows.

import { useCallback, useRef, useState, type ReactNode } from "react";
import { Modal, Pressable, ScrollView, Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { ApiError } from "./outpost";
import { Button, C, Chip, ErrorText, Field, P, s } from "./ui";

export function message(e: unknown, fallback = "Something went wrong.") {
  if (e instanceof ApiError || e instanceof Error) return e.message;
  return fallback;
}

/** A full-screen page over the app, with its title, a way back, and room for the status bar. */
export function Screen({
  title,
  onClose,
  back,
  children,
  scroll = true,
}: {
  title: string;
  onClose: () => void;
  /** Shown instead of Close (e.g. "‹ Personas"); Android's back does the same. */
  back?: string;
  children: ReactNode;
  scroll?: boolean;
}) {
  const insets = useSafeAreaInsets();
  return (
    <Modal visible animationType="slide" onRequestClose={onClose}>
      <View style={[s.screen, { paddingTop: insets.top + 8, paddingBottom: insets.bottom }]}>
        <View style={[s.row, { paddingHorizontal: 16, paddingBottom: 8 }]}>
          <Text style={{ color: C.ink, fontSize: 20, fontWeight: "700", flex: 1 }} numberOfLines={1}>
            {title}
          </Text>
          <Chip label={back ?? "Close"} onPress={onClose} />
        </View>
        {scroll ? (
          <ScrollView contentContainerStyle={[s.pad, { paddingBottom: 40 }]} keyboardShouldPersistTaps="handled">
            {children}
          </ScrollView>
        ) : (
          children
        )}
      </View>
    </Modal>
  );
}

/** A setting that's on or off, with what it does underneath. */
export function Toggle({ title, hint, on, onChange }: { title: string; hint?: string; on: boolean; onChange: (on: boolean) => void }) {
  return (
    <Pressable
      accessibilityRole="switch"
      accessibilityState={{ checked: on }}
      onPress={() => onChange(!on)}
      style={[s.row, { alignItems: "flex-start", paddingVertical: 4 }]}
    >
      <View style={{ flex: 1, gap: 2 }}>
        <Text style={{ color: C.ink, fontSize: 15 }}>{title}</Text>
        {hint ? <Text style={{ color: C.muted, fontSize: 13 }}>{hint}</Text> : null}
      </View>
      <View
        style={{
          width: 46,
          height: 28,
          borderRadius: 14,
          padding: 3,
          backgroundColor: on ? C.ember : C.raised,
          alignItems: on ? "flex-end" : "flex-start",
        }}
      >
        <View style={{ width: 22, height: 22, borderRadius: 11, backgroundColor: on ? C.onEmber : C.muted }} />
      </View>
    </Pressable>
  );
}

export function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <View style={{ gap: 8 }}>
      <Text style={{ color: C.soft, fontSize: 13, fontWeight: "700", textTransform: "uppercase", letterSpacing: 0.6 }}>{title}</Text>
      {children}
    </View>
  );
}

/**
 * Actions the server only takes from a paired phone with the PIN again: `guard(run)` runs
 * it, and if the server asks for the PIN, asks for it here and runs it once more with it.
 * Render `prompt` somewhere on the screen.
 */
export function usePin() {
  const [asking, setAsking] = useState<string | null>(null);
  const [pin, setPin] = useState("");
  const answer = useRef<((pin: string | null) => void) | null>(null);

  const ask = useCallback((why: string) => {
    setPin("");
    setAsking(why);
    return new Promise<string | null>((resolve) => {
      answer.current = resolve;
    });
  }, []);
  const finish = (value: string | null) => {
    setAsking(null);
    answer.current?.(value);
    answer.current = null;
  };

  const guard = useCallback(
    async <T,>(run: (headers?: Record<string, string>) => Promise<T>): Promise<T | null> => {
      try {
        return await run();
      } catch (e) {
        if (!(e instanceof ApiError && e.code === "pin_required")) throw e;
        const given = await ask(e.message);
        if (!given) return null;
        return run({ "X-Outpost-Pin": given });
      }
    },
    [ask],
  );

  const prompt = (
    <Modal visible={asking !== null} transparent animationType="fade" onRequestClose={() => finish(null)}>
      <View style={{ flex: 1, backgroundColor: "rgba(0,0,0,0.6)", justifyContent: "center", padding: 24 }}>
        <View style={[s.card, { gap: 12 }]}>
          <Text style={{ color: C.ink, fontSize: 18, fontWeight: "600" }}>Your PIN</Text>
          <P muted>{asking ?? ""}</P>
          <Field
            value={pin}
            onChangeText={setPin}
            secureTextEntry
            autoFocus
            placeholder="PIN (or password)"
            onSubmitEditing={() => pin.trim() && finish(pin.trim())}
          />
          <View style={s.row}>
            <Button kind="ghost" title="Cancel" onPress={() => finish(null)} style={{ flex: 1 }} />
            <Button title="OK" disabled={!pin.trim()} onPress={() => finish(pin.trim())} style={{ flex: 1 }} />
          </View>
        </View>
      </View>
    </Modal>
  );
  return { guard, prompt };
}

/** A small "loading / error / empty" line for lists. */
export function ListState({ loading, error, empty, emptyText }: { loading: boolean; error: string | null; empty: boolean; emptyText: string }) {
  if (error) return <ErrorText>{error}</ErrorText>;
  if (loading) return <P muted>Loading…</P>;
  if (empty) return <P muted>{emptyText}</P>;
  return null;
}

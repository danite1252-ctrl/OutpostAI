import type { ReactNode } from "react";
import { ActivityIndicator, Pressable, StyleSheet, Text, TextInput, View, type TextInputProps, type ViewStyle } from "react-native";

/** Ember, the desktop app's palette. */
export const C = {
  deep: "#0c0a0d",
  night: "#121013",
  surface: "#1b181c",
  raised: "#262127",
  line: "#383238",
  ink: "#f3eee8",
  soft: "#d9d0c8",
  muted: "#a9a09a",
  ember: "#ff6a3d",
  amber: "#ffb547",
  onEmber: "#1e0c05",
  ok: "#7bd88f",
  danger: "#ff8a9a",
  pick: "#6cb4ff",
  warn: "#ff9a3c",
};

export function Button({
  title,
  onPress,
  kind = "primary",
  disabled,
  busy,
  style,
}: {
  title: string;
  onPress: () => void;
  kind?: "primary" | "ghost" | "danger";
  disabled?: boolean;
  busy?: boolean;
  style?: ViewStyle;
}) {
  const bg = kind === "primary" ? C.ember : "transparent";
  const fg = kind === "primary" ? C.onEmber : kind === "danger" ? C.danger : C.ink;
  return (
    <Pressable
      accessibilityRole="button"
      onPress={onPress}
      disabled={disabled || busy}
      style={({ pressed }) => [
        s.button,
        { backgroundColor: bg, borderWidth: kind === "primary" ? 0 : 1, opacity: disabled ? 0.45 : pressed ? 0.8 : 1 },
        style,
      ]}
    >
      {busy ? <ActivityIndicator color={fg} /> : <Text style={[s.buttonText, { color: fg }]}>{title}</Text>}
    </Pressable>
  );
}

export function Field(props: TextInputProps & { label?: string }) {
  const { label, style, ...rest } = props;
  return (
    <View style={{ gap: 6 }}>
      {label ? <Text style={s.label}>{label}</Text> : null}
      <TextInput placeholderTextColor={C.muted} style={[s.input, style]} {...rest} />
    </View>
  );
}

export function H1({ children }: { children: ReactNode }) {
  return <Text style={s.h1}>{children}</Text>;
}

export function P({ children, muted, style }: { children: ReactNode; muted?: boolean; style?: object }) {
  return <Text style={[s.p, muted && { color: C.muted }, style]}>{children}</Text>;
}

export function ErrorText({ children }: { children: ReactNode }) {
  if (!children) return null;
  return <Text style={s.error}>{children}</Text>;
}

export function Avatar({ name, color, size = 32 }: { name: string; color: string; size?: number }) {
  return (
    <View style={{ width: size, height: size, borderRadius: size / 2, backgroundColor: color, alignItems: "center", justifyContent: "center" }}>
      <Text style={{ color: C.onEmber, fontWeight: "700", fontSize: size * 0.42 }}>{name.trim().charAt(0).toUpperCase() || "?"}</Text>
    </View>
  );
}

export function Chip({ label, active, onPress, color }: { label: string; active?: boolean; onPress: () => void; color?: string }) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityState={{ selected: active }}
      onPress={onPress}
      style={[s.chip, active && { borderColor: C.ember, backgroundColor: C.raised }]}
    >
      <Text style={{ color: color ?? (active ? C.ink : C.muted), fontSize: 14 }}>{label}</Text>
    </Pressable>
  );
}

export function Bar({ fraction, color = C.ember }: { fraction: number | null; color?: string }) {
  return (
    <View style={s.track}>
      <View style={{ height: 6, borderRadius: 3, backgroundColor: color, width: `${Math.round((fraction ?? 0.3) * 100)}%`, opacity: fraction == null ? 0.5 : 1 }} />
    </View>
  );
}

export const s = StyleSheet.create({
  screen: { flex: 1, backgroundColor: C.night },
  pad: { padding: 16, gap: 14 },
  card: { backgroundColor: C.surface, borderColor: C.line, borderWidth: 1, borderRadius: 16, padding: 14, gap: 8 },
  button: { minHeight: 48, borderRadius: 14, paddingHorizontal: 16, alignItems: "center", justifyContent: "center", borderColor: C.line },
  buttonText: { fontSize: 16, fontWeight: "600" },
  input: { minHeight: 48, borderRadius: 12, borderWidth: 1, borderColor: C.line, backgroundColor: C.deep, color: C.ink, paddingHorizontal: 12, fontSize: 16 },
  label: { color: C.muted, fontSize: 14 },
  h1: { color: C.ink, fontSize: 24, fontWeight: "700" },
  p: { color: C.ink, fontSize: 15, lineHeight: 21 },
  error: { color: C.danger, fontSize: 14 },
  chip: { minHeight: 40, paddingHorizontal: 12, borderRadius: 10, borderWidth: 1, borderColor: C.line, justifyContent: "center" },
  track: { height: 6, borderRadius: 3, backgroundColor: C.raised, overflow: "hidden" },
  row: { flexDirection: "row", alignItems: "center", gap: 10 },
});

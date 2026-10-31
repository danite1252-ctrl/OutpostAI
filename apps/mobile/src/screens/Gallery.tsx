import { useCallback, useEffect, useState } from "react";
import { Alert, AppState, Dimensions, Image, Modal, Pressable, ScrollView, Text, TextInput, View } from "react-native";
import { useVideoPlayer, VideoView } from "expo-video";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { WebView } from "react-native-webview";
import OutpostNet from "../../modules/outpost-net";
import { VIEWER_HTML } from "../viewer-html";
import { AuthedImage, dropImage, useAuthedImage } from "../images";
import type { Account, Media, Outpost } from "../outpost";
import { Avatar, Button, C, Chip, ErrorText, H1, P, s } from "../ui";

/** Mine, the shared gallery, or (owner only) another account's. */
type Tab = "mine" | "shared" | Account;

export default function Gallery({
  api,
  me,
  onError,
  onUse,
}: {
  api: Outpost;
  me: Account;
  onError: (e: unknown) => void;
  onUse: (dataUri: string, task: "edit" | "animate" | "model") => void;
}) {
  const [tab, setTab] = useState<Tab>("mine");
  const isOwner = me.role === "owner";
  const [others, setOthers] = useState<Account[]>([]);
  const [menu, setMenu] = useState(false);
  const other = typeof tab === "object" ? tab : null;

  useEffect(() => {
    if (!isOwner) return;
    api.get<Account[]>("/api/accounts").then(
      (list) => setOthers(list.filter((a) => a.id !== me.id && a.role !== "service")),
      () => undefined,
    );
  }, [api, isOwner, me.id]);
  const [items, setItems] = useState<Media[]>([]);
  const [viewing, setViewing] = useState<Media | null>(null);
  const [error, setError] = useState<string | null>(null);
  // Searching my gallery by what's in the pictures.
  const [query, setQuery] = useState("");
  const [results, setResults] = useState<Media[] | null>(null);
  useEffect(() => {
    const q = query.trim();
    if (!q || tab !== "mine") {
      setResults(null);
      return;
    }
    const t = setTimeout(() => {
      api.get<Media[]>(`/api/gallery/search?q=${encodeURIComponent(q)}`).then(setResults, (e) => {
        onError(e);
        setError(e instanceof Error ? e.message : "Couldn't search.");
      });
    }, 400);
    return () => clearTimeout(t);
  }, [api, query, tab, onError]);
  const shown = results ?? items;

  const path =
    tab === "mine"
      ? "/api/gallery?limit=60"
      : tab === "shared"
        ? "/api/gallery/shared?limit=60"
        : `/api/gallery?limit=60&account_id=${encodeURIComponent(tab.id)}`;
  const load = useCallback(() => {
    setError(null);
    setItems([]);
    api.get<Media[]>(path).then(setItems, (e) => {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't load the gallery.");
    });
  }, [api, path, onError]);
  useEffect(load, [load]);

  // New pictures, videos and models appear by themselves while the gallery is open.
  useEffect(() => {
    const t = setInterval(() => {
      if (AppState.currentState !== "active") return;
      api.get<Media[]>(path).then(
        (page) =>
          setItems((have) => {
            const seen = new Set(have.map((m) => m.id));
            const fresh = page.filter((m) => !seen.has(m.id));
            return fresh.length ? [...fresh, ...have] : have;
          }),
        () => undefined,
      );
    }, 8000);
    return () => clearInterval(t);
  }, [api, path]);

  const size = Math.floor((Dimensions.get("window").width - 16 * 2 - 8 * 2) / 3);

  async function share(m: Media, shared: boolean) {
    try {
      const updated = await api.json<Media>("POST", `/api/media/${m.id}/share`, { shared });
      setViewing({ ...m, shared_at: updated.shared_at });
      load();
    } catch (e) {
      onError(e);
      setError(e instanceof Error ? e.message : "Couldn't change sharing.");
    }
  }

  function remove(m: Media) {
    const whose = m.account_id === me.id ? "your gallery" : `${m.by_name ?? other?.name ?? "their"}'s gallery`;
    Alert.alert("Delete this?", `It goes from ${whose} on the PC${m.shared_at ? ", and from the shared gallery" : ""}.`, [
      { text: "Keep", style: "cancel" },
      {
        text: "Delete",
        style: "destructive",
        onPress: async () => {
          try {
            await api.json("DELETE", `/api/media/${m.id}`);
            dropImage(`/api/media/${m.id}`);
            setViewing(null);
            load();
          } catch (e) {
            onError(e);
          }
        },
      },
    ]);
  }

  return (
    <View style={s.screen}>
      <ScrollView contentContainerStyle={s.pad}>
        <H1>Gallery</H1>
        <View style={[s.row, { flexWrap: "wrap" }]}>
          <Chip label="Mine" active={tab === "mine"} onPress={() => setTab("mine")} />
          <Chip label="Shared" active={tab === "shared"} onPress={() => setTab("shared")} />
          {isOwner && others.length > 0 && (
            <Chip label={other ? `${other.name} ▾` : "Other users ▾"} active={!!other || menu} onPress={() => setMenu((m) => !m)} />
          )}
        </View>
        {menu && (
          <View style={[s.card, { paddingVertical: 4 }]}>
            {others.map((a) => (
              <Pressable
                key={a.id}
                accessibilityRole="button"
                onPress={() => {
                  setTab(a);
                  setMenu(false);
                }}
                style={({ pressed }) => [s.row, { minHeight: 48 }, pressed && { opacity: 0.7 }]}
              >
                <Avatar name={a.name} color={a.avatar_color} size={28} />
                <Text style={{ color: other?.id === a.id ? C.ember : C.ink, fontSize: 16 }}>{a.name}</Text>
              </Pressable>
            ))}
          </View>
        )}
        {tab === "mine" && (
          <TextInput
            value={query}
            onChangeText={setQuery}
            placeholder="Search, e.g. dog on a beach"
            placeholderTextColor={C.muted}
            accessibilityLabel="Search my gallery"
            returnKeyType="search"
            style={[s.card, { color: C.ink, fontSize: 16, paddingVertical: 10 }]}
          />
        )}
        {results && results.length === 0 && <P muted>Nothing matches “{query.trim()}”.</P>}
        {other && <P muted>{other.name}'s gallery. Only owners can see this.</P>}
        <ErrorText>{error}</ErrorText>
        {!results && items.length === 0 && (
          <P muted>
            {tab === "mine"
              ? "Nothing yet. What you make in Create lands here."
              : tab === "shared"
                ? "Nothing shared yet."
                : `${other?.name} hasn't made anything yet.`}
          </P>
        )}
        <View style={{ flexDirection: "row", flexWrap: "wrap", gap: 8 }}>
          {shown.map((m) => (
            <Pressable key={m.id} accessibilityRole="button" accessibilityLabel={m.prompt ?? "Picture"} onPress={() => setViewing(m)}>
              {m.kind === "model" ? (
                <View style={{ width: size, height: size, borderRadius: 10, backgroundColor: C.raised, alignItems: "center", justifyContent: "center" }}>
                  <Text style={{ color: C.soft, fontSize: 26 }}>⬡</Text>
                  <Text style={{ color: C.muted, fontSize: 12 }}>3D model</Text>
                </View>
              ) : m.kind === "video" ? (
                <View style={{ width: size, height: size, borderRadius: 10, backgroundColor: C.raised, alignItems: "center", justifyContent: "center" }}>
                  <Text style={{ color: C.muted }}>▶ Video</Text>
                </View>
              ) : (
                <AuthedImage api={api} path={`/api/media/${m.id}`} style={{ width: size, height: size, borderRadius: 10 }} />
              )}
              {tab === "shared" && (
                <Text numberOfLines={1} style={{ color: C.muted, fontSize: 11, width: size }}>
                  {m.account_id === me.id ? "You" : m.by_name}
                </Text>
              )}
            </Pressable>
          ))}
        </View>
      </ScrollView>
      {viewing && (
        <Viewer
          api={api}
          media={viewing}
          mine={viewing.account_id === me.id}
          canDelete={viewing.account_id === me.id || isOwner}
          canUnshare={viewing.account_id === me.id || isOwner}
          by={viewing.account_id === me.id ? undefined : (viewing.by_name ?? other?.name)}
          onClose={() => setViewing(null)}
          onShare={(shared) => void share(viewing, shared)}
          onDelete={() => remove(viewing)}
          onUse={(uri, task) => {
            setViewing(null);
            onUse(uri, task);
          }}
        />
      )}
    </View>
  );
}

function Viewer({
  api,
  media: m,
  mine,
  canDelete,
  canUnshare,
  by,
  onClose,
  onShare,
  onDelete,
  onUse,
}: {
  api: Outpost;
  media: Media;
  mine: boolean;
  canDelete: boolean;
  canUnshare: boolean;
  /** Who made it, when it isn't yours. */
  by?: string;
  onClose: () => void;
  onShare: (shared: boolean) => void;
  onDelete: () => void;
  onUse: (dataUri: string, task: "edit" | "animate" | "model") => void;
}) {
  const { uri } = useAuthedImage(api, m.kind === "image" ? `/api/media/${m.id}` : null);
  const [touching, setTouching] = useState(false);
  const w = Dimensions.get("window").width;
  // The viewer covers the whole screen, status bar included: keep clear of it.
  const insets = useSafeAreaInsets();
  return (
    <Modal visible animationType="slide" onRequestClose={onClose} transparent={false}>
      <ScrollView
        scrollEnabled={!touching}
        style={{ backgroundColor: C.deep }}
        contentContainerStyle={[s.pad, { paddingTop: insets.top + 12, paddingBottom: insets.bottom + 16 }]}
      >
        {/* A header row keeps the picture or video clear of the top of the screen. */}
        <View style={[s.row, { marginBottom: 8 }]}>
          <Pressable accessibilityRole="button" accessibilityLabel="Close" onPress={onClose} style={[s.chip, { minHeight: 40 }]}>
            <Text style={{ color: C.ink }}>‹ Back</Text>
          </Pressable>
          <Text style={{ color: C.muted, flex: 1 }}>{m.kind === "video" ? "Video" : m.kind === "model" ? "3D model" : "Picture"}</Text>
        </View>
        {m.kind === "model" ? (
          <ModelViewer api={api} media={m} width={w - 32} onTouching={setTouching} />
        ) : m.kind === "image" ? (
          uri ? (
            <Image source={{ uri }} style={{ width: w - 32, height: w - 32, borderRadius: 12 }} resizeMode="contain" />
          ) : (
            <View style={{ width: w - 32, height: w - 32, borderRadius: 12, backgroundColor: C.raised }} />
          )
        ) : (
          <Video api={api} media={m} width={w - 32} />
        )}
        {m.kind !== "model" && <P>{m.prompt || "No prompt"}</P>}
        <P muted>
          {by ? `By ${by} · ` : ""}
          seed {m.seed ?? "—"} · {new Date(m.created_at * 1000).toLocaleString()}
          {m.shared_at ? " · shared" : ""}
        </P>
        {uri && (
          <>
            <Button kind="ghost" title="Edit this picture" onPress={() => onUse(uri, "edit")} />
            <Button kind="ghost" title="Animate this picture" onPress={() => onUse(uri, "animate")} />
            <Button kind="ghost" title="Make a 3D model" onPress={() => onUse(uri, "model")} />
          </>
        )}
        {m.kind === "model" ? (
          <>
            <SaveButton api={api} path={`/api/media/${m.id}/print`} name={`outpost-${m.id.slice(0, 8)}.3mf`} mime="model/3mf" label="for 3D printing (.3mf)" primary />
            <SaveButton api={api} path={`/api/media/${m.id}`} name={`outpost-${m.id.slice(0, 8)}.glb`} mime="model/gltf-binary" label="for games and 3D apps (.glb)" />
          </>
        ) : (
          <SaveButton
            api={api}
            path={`/api/media/${m.id}`}
            name={`outpost-${m.id.slice(0, 8)}.${m.mime === "image/jpeg" ? "jpg" : (m.mime.split("/")[1] ?? "png")}`}
            mime={m.mime}
            label={m.kind === "video" ? "to the phone's videos" : "to the phone's pictures"}
          />
        )}
        {(mine || (canUnshare && m.shared_at)) && (
          <Button kind="ghost" title={m.shared_at ? "Stop sharing" : "Share with everyone"} onPress={() => onShare(!m.shared_at)} />
        )}
        {canDelete && <Button kind="danger" title="Delete" onPress={onDelete} />}
        <Button title="Close" onPress={onClose} />
      </ScrollView>
    </Modal>
  );
}

/** A 3D model, turned with a finger (three.js in a WebView, reading the downloaded file). */
function ModelViewer({ api, media: m, width, onTouching }: { api: Outpost; media: Media; width: number; onTouching: (on: boolean) => void }) {
  const [file, setFile] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [ready, setReady] = useState(false);
  useEffect(() => {
    let live = true;
    api.download(`/api/media/${m.id}`, `${m.id}.glb`).then(
      (u) => live && setFile(u),
      (e) => live && setError(e instanceof Error ? e.message : "Couldn't load the model."),
    );
    return () => {
      live = false;
    };
  }, [api, m.id]);
  const box = { width, height: width, borderRadius: 12, overflow: "hidden" as const, backgroundColor: C.raised };
  if (error) return <ErrorText>{error}</ErrorText>;
  return (
    // While a finger is on the model, the page doesn't scroll (the drag turns the model).
    <View style={box} onTouchStart={() => onTouching(true)} onTouchEnd={() => onTouching(false)} onTouchCancel={() => onTouching(false)}>
      {file && (
        <WebView
          source={{ html: VIEWER_HTML, baseUrl: "file:///" }}
          originWhitelist={["*"]}
          allowFileAccess
          allowFileAccessFromFileURLs
          allowUniversalAccessFromFileURLs
          injectedJavaScriptBeforeContentLoaded={`window.OUTPOST_MODEL=${JSON.stringify(file)};window.OUTPOST_BG="${C.raised}";true;`}
          onMessage={(e) => {
            try {
              const msg = JSON.parse(e.nativeEvent.data) as { type: string; message?: string };
              if (msg.type === "ready") setReady(true);
              if (msg.type === "error") setError(msg.message ?? "This model couldn't be shown.");
            } catch {
              /* not ours */
            }
          }}
          style={{ flex: 1, backgroundColor: C.raised }}
          scrollEnabled={false}
          overScrollMode="never"
          accessibilityLabel="3D model: drag to turn it, pinch to zoom"
        />
      )}
      {!ready && (
        <View style={{ position: "absolute", left: 0, right: 0, top: 0, bottom: 0, alignItems: "center", justifyContent: "center" }} pointerEvents="none">
          <Text style={{ color: C.muted }}>Loading the model…</Text>
        </View>
      )}
    </View>
  );
}

/** Download a gallery item and copy it into the phone's own folders, then offer to share it. */
function SaveButton({ api, path, name, mime, label, primary }: { api: Outpost; path: string; name: string; mime: string; label: string; primary?: boolean }) {
  const [busy, setBusy] = useState(false);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const folder = mime.startsWith("image/") ? "Pictures/Outpost" : mime.startsWith("video/") ? "Movies/Outpost" : "Download/Outpost";
  const save = async () => {
    setBusy(true);
    setError(null);
    try {
      const file = await api.download(path, name);
      setSaved(await OutpostNet.saveToPhone(file, name, mime));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Couldn't save it.");
    } finally {
      setBusy(false);
    }
  };
  if (saved) {
    return (
      <View style={{ gap: 6 }}>
        <P muted>Saved in {folder}.</P>
        <Button kind="ghost" title="Share it…" onPress={() => OutpostNet.shareSaved(saved, mime, "Send it to").catch((e) => setError(String(e?.message ?? e)))} />
        {error && <ErrorText>{error}</ErrorText>}
      </View>
    );
  }
  return (
    <View style={{ gap: 6 }}>
      <Button kind={primary ? "primary" : "ghost"} title={busy ? "Saving…" : `Save ${label}`} disabled={busy} onPress={() => void save()} />
      {error && <ErrorText>{error}</ErrorText>}
    </View>
  );
}

/** Videos are saved on the phone first (through the pinned connection), then played. */
function Video({ api, media: m, width }: { api: Outpost; media: Media; width: number }) {
  const [uri, setUri] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const ext = m.mime.includes("webm") ? "webm" : "mp4";
    api.download(`/api/media/${m.id}`, `${m.id}.${ext}`).then(
      (u) => live && setUri(u),
      (e) => live && setError(e instanceof Error ? e.message : "Couldn't load the video."),
    );
    return () => {
      live = false;
    };
  }, [api, m.id, m.mime]);
  if (error) return <ErrorText>{error}</ErrorText>;
  if (!uri) {
    return (
      <View style={{ width, height: width * 0.5625, borderRadius: 12, backgroundColor: C.raised, alignItems: "center", justifyContent: "center" }}>
        <Text style={{ color: C.muted }}>Loading the video…</Text>
      </View>
    );
  }
  return <Player uri={uri} width={width} />;
}

function Player({ uri, width }: { uri: string; width: number }) {
  const player = useVideoPlayer(uri, (p) => {
    p.loop = true;
    p.play();
  });
  // Size the player to the clip (landscape or portrait), so nothing is cropped.
  const [aspect, setAspect] = useState(16 / 9);
  useEffect(() => {
    const sub = player.addListener("sourceLoad", (e) => {
      const size = e.availableVideoTracks[0]?.size;
      if (size && size.width > 0 && size.height > 0) setAspect(size.width / size.height);
    });
    return () => sub.remove();
  }, [player]);
  const height = Math.min(width / aspect, Dimensions.get("window").height * 0.6);
  return <VideoView player={player} style={{ width, height, borderRadius: 12, backgroundColor: "#000" }} nativeControls allowsFullscreen contentFit="contain" />;
}

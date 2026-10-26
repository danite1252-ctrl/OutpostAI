import { useEffect, useState } from "react";
import { ActivityIndicator, Image, View, type ImageStyle } from "react-native";
import type { Outpost } from "./outpost";
import { C } from "./ui";

/** Pictures come through the pinned connection, so they're fetched here, not by <Image>. */
const cache = new Map<string, string>();
const LIMIT = 60;

export function useAuthedImage(api: Outpost, path: string | null) {
  const [uri, setUri] = useState<string | null>(path ? (cache.get(path) ?? null) : null);
  const [failed, setFailed] = useState(false);
  useEffect(() => {
    if (!path) return;
    const hit = cache.get(path);
    if (hit) {
      setUri(hit);
      return;
    }
    let live = true;
    setFailed(false);
    api.image(path).then(
      (u) => {
        cache.set(path, u);
        if (cache.size > LIMIT) cache.delete(cache.keys().next().value as string);
        if (live) setUri(u);
      },
      () => live && setFailed(true),
    );
    return () => {
      live = false;
    };
  }, [api, path]);
  return { uri, failed };
}

export function dropImage(path: string) {
  cache.delete(path);
}

export function AuthedImage({ api, path, style }: { api: Outpost; path: string; style: ImageStyle }) {
  const { uri, failed } = useAuthedImage(api, path);
  if (!uri) {
    return (
      <View style={[style, { alignItems: "center", justifyContent: "center", backgroundColor: C.raised }]}>
        {!failed && <ActivityIndicator color={C.muted} />}
      </View>
    );
  }
  return <Image source={{ uri }} style={style} resizeMode="cover" />;
}

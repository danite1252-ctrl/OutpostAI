// "Send to Outpost": what to do with something shared from another app (Android's share
// sheet): text and links start a chat; pictures go to chat, Create or the gallery;
// documents become a chat about them, or just join my documents.

import { useState } from "react";
import { Image, ScrollView, Text, View } from "react-native";
import OutpostNet, { type Shared, type SharedFile } from "../../modules/outpost-net";
import type { Outpost } from "../outpost";
import { Button, C, ErrorText, H1, P, s } from "../ui";

/** What a chat should start with. */
export interface ChatStart {
  text: string;
  /** Data URIs. */
  images: string[];
  documentIds: string[];
}

const DOC = /\.(pdf|docx|txt|md|markdown|csv|html?)$/i;
const MAX_CHAT_IMAGES = 4;

function kinds(files: SharedFile[]) {
  const usable = files.filter((f) => f.path);
  return {
    images: usable.filter((f) => f.mime?.startsWith("image/") && f.dataUri),
    videos: usable.filter((f) => f.mime?.startsWith("video/")),
    // A text file shared as a file is a document; shared text (not a file) is just text.
    docs: usable.filter((f) => !f.mime?.startsWith("image/") && !f.mime?.startsWith("video/") && DOC.test(f.name)),
    skipped: files.filter((f) => !f.path || (!f.mime?.startsWith("image/") && !f.mime?.startsWith("video/") && !DOC.test(f.name))),
  };
}

export default function Share({
  api,
  shared,
  onChat,
  onCreate,
  onDone,
}: {
  api: Outpost;
  shared: Shared;
  onChat: (start: ChatStart) => void;
  onCreate: (image: string, task: "edit" | "animate" | "model") => void;
  onDone: () => void;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [done, setDone] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const files = shared.files ?? [];
  const { images, videos, docs, skipped } = kinds(files);
  const text = [shared.subject, shared.text].filter(Boolean).join("\n");

  const finish = () => {
    OutpostNet.clearShared();
    onDone();
  };

  async function run(label: string, work: () => Promise<void>) {
    setBusy(label);
    setError(null);
    try {
      await work();
    } catch (e) {
      setError(e instanceof Error ? e.message : "That didn't work.");
    } finally {
      setBusy(null);
    }
  }

  /** Each document into my documents; their ids. */
  async function addDocs(): Promise<string[]> {
    const ids: string[] = [];
    for (const [i, d] of docs.entries()) {
      setBusy(`Sending ${d.name}${docs.length > 1 ? ` (${i + 1} of ${docs.length})` : ""}…`);
      const r = await api.upload<{ document: { id: string } }>(
        `/api/documents?name=${encodeURIComponent(d.name)}`,
        d.path!,
        "application/octet-stream",
      );
      ids.push(r.document.id);
    }
    return ids;
  }

  async function saveToGallery() {
    const items = [...images, ...videos];
    for (const [i, f] of items.entries()) {
      setBusy(`Saving ${i + 1} of ${items.length}…`);
      await api.upload(`/api/media/import?name=${encodeURIComponent(f.name)}`, f.path!, "application/octet-stream");
    }
    setDone(items.length === 1 ? "Saved to your gallery." : `${items.length} saved to your gallery.`);
  }

  const what =
    docs.length > 0
      ? docs.length === 1
        ? docs[0].name
        : `${docs.length} documents`
      : images.length + videos.length > 0
        ? `${images.length + videos.length} ${images.length + videos.length === 1 ? (videos.length ? "video" : "picture") : "items"}`
        : "Some text";

  return (
    <ScrollView style={s.screen} contentContainerStyle={s.pad}>
      <H1>Send to Outpost</H1>
      <P muted>{what}</P>
      {images.length > 0 && (
        <View style={[s.row, { flexWrap: "wrap" }]}>
          {images.slice(0, 6).map((f) => (
            <Image key={f.path} source={{ uri: f.dataUri! }} style={{ width: 96, height: 96, borderRadius: 10 }} />
          ))}
        </View>
      )}
      {text ? (
        <Text numberOfLines={6} style={[s.card, { color: C.soft }]}>
          {text}
        </Text>
      ) : null}
      {skipped.length > 0 && (
        <P muted>
          Left out: {skipped.map((f) => `${f.name}${f.error ? ` (${f.error})` : ""}`).join(", ")}. Outpost takes pictures, videos, PDFs, Word
          files and text.
        </P>
      )}

      {done ? (
        <>
          <P>{done}</P>
          <Button title="Done" onPress={finish} />
        </>
      ) : (
        <View style={{ gap: 10 }}>
          {docs.length > 0 && (
            <>
              <Button
                title={docs.length === 1 ? "Ask about it in a new chat" : "Ask about them in a new chat"}
                busy={!!busy}
                onPress={() =>
                  void run("docs", async () => {
                    const documentIds = await addDocs();
                    OutpostNet.clearShared();
                    onChat({ text, images: [], documentIds });
                  })
                }
              />
              <Button
                title="Just add to my documents"
                kind="ghost"
                disabled={!!busy}
                onPress={() =>
                  void run("docs", async () => {
                    await addDocs();
                    setDone(
                      docs.length === 1
                        ? `${docs[0].name} is in your documents. Use it from a chat's Documents.`
                        : "They're in your documents. Use them from a chat's Documents.",
                    );
                  })
                }
              />
            </>
          )}
          {docs.length === 0 && images.length > 0 && (
            <Button
              title="Ask about it in chat"
              busy={!!busy}
              onPress={() => {
                OutpostNet.clearShared();
                onChat({ text, images: images.slice(0, MAX_CHAT_IMAGES).map((f) => f.dataUri!), documentIds: [] });
              }}
            />
          )}
          {docs.length === 0 && images.length === 1 && (
            <>
              <Button title="Edit it in Create" kind="ghost" disabled={!!busy} onPress={() => onCreate(images[0].dataUri!, "edit")} />
              <Button title="Animate it" kind="ghost" disabled={!!busy} onPress={() => onCreate(images[0].dataUri!, "animate")} />
              <Button title="Make a 3D model" kind="ghost" disabled={!!busy} onPress={() => onCreate(images[0].dataUri!, "model")} />
            </>
          )}
          {docs.length === 0 && images.length + videos.length > 0 && (
            <Button title="Save to my gallery" kind="ghost" disabled={!!busy} onPress={() => void run("gallery", saveToGallery)} />
          )}
          {files.length === 0 && text ? <Button title="Start a chat with it" onPress={() => onChat({ text, images: [], documentIds: [] })} /> : null}
          {busy && busy.length > 8 ? <P muted>{busy}</P> : null}
          <ErrorText>{error}</ErrorText>
          <Button title="Cancel" kind="ghost" disabled={!!busy} onPress={finish} />
        </View>
      )}
    </ScrollView>
  );
}

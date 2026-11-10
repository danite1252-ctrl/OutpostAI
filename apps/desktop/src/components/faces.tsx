// Known faces (owners): people Outpost can name at the door. A name, maybe their Outpost
// account, and a few photos; their faces stay on the server.

import { useCallback, useEffect, useState } from "react";
import { api, ApiError, request, type Account } from "../api";
import { useAuthedUrl } from "./images";
import { Button, ErrorNote, areYouSure } from "./ui";

interface Sample {
  id: string;
  created_at: number;
}
interface KnownFace {
  id: string;
  name: string;
  account_id: string | null;
  samples: Sample[];
}
interface FacesState {
  faces: KnownFace[];
  setup: { ready: boolean; busy: boolean; error: string | null };
  voice_installed: boolean;
}

function message(e: unknown) {
  return e instanceof ApiError ? e.message : e instanceof Error ? e.message : String(e);
}

function asDataUrl(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = new FileReader();
    r.onload = () => resolve(String(r.result));
    r.onerror = () => reject(new Error(`${file.name} couldn't be read.`));
    r.readAsDataURL(file);
  });
}

function Thumb({ token, sample, onRemove }: { token: string; sample: Sample; onRemove: () => void }) {
  const { src } = useAuthedUrl(token, `/api/faces/samples/${sample.id}`);
  return (
    <div className="group relative h-16 w-16 overflow-hidden rounded-lg border border-line bg-deep">
      {src && <img src={src} alt="" className="h-full w-full object-cover" />}
      <button
        type="button"
        aria-label="Remove this face"
        onClick={onRemove}
        className="absolute right-0.5 top-0.5 rounded bg-black/70 px-1 text-xs text-ink opacity-0 group-hover:opacity-100 focus:opacity-100"
      >
        ×
      </button>
    </div>
  );
}

export function KnownFacesPanel({ token }: { token: string }) {
  const [state, setState] = useState<FacesState | null>(null);
  const [accounts, setAccounts] = useState<Account[]>([]);
  const [name, setName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [notes, setNotes] = useState<Record<string, string>>({});
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setState(await request<FacesState>("GET", "/api/faces", { token }));
    } catch (e) {
      setError(message(e));
    }
  }, [token]);
  useEffect(() => {
    void load();
    api.listAccounts(token).then(setAccounts, () => undefined);
  }, [load, token]);
  // While it sets up, look again every few seconds.
  useEffect(() => {
    if (!state?.setup.busy) return;
    const t = setInterval(() => void load(), 3000);
    return () => clearInterval(t);
  }, [state?.setup.busy, load]);

  async function run(key: string, work: () => Promise<unknown>) {
    setBusy(key);
    setError(null);
    try {
      await work();
      await load();
    } catch (e) {
      setError(message(e));
    } finally {
      setBusy(null);
    }
  }

  async function addPhotos(face: KnownFace, files: FileList | null) {
    if (!files?.length) return;
    await run(face.id, async () => {
      const said: string[] = [];
      for (const f of Array.from(files)) {
        try {
          const r = await request<{ note: string | null }>("POST", `/api/faces/${face.id}/photos`, {
            token,
            body: { image: await asDataUrl(f) },
          });
          if (r.note) said.push(`${f.name}: ${r.note}`);
        } catch (e) {
          said.push(`${f.name}: ${message(e)}`);
        }
      }
      setNotes((n) => ({ ...n, [face.id]: said.join("\n") }));
    });
  }

  if (!state) return <ErrorNote>{error}</ErrorNote>;
  const select = "min-h-9 rounded-lg border border-line bg-surface px-2 text-sm text-ink";

  return (
    <div className="flex flex-col gap-4 text-sm">
      <p className="text-muted">
        People Outpost can name at the door, in routines with <strong>Say who it is</strong> on. Add a name and 5 to 10 clear photos of
        their face (different angles; a couple taken at your door help most). Faces stay on your server and are never sent anywhere.
        Only add people who are happy to be recognised; anyone else is just described.
      </p>
      <ErrorNote>{error}</ErrorNote>

      {!state.setup.ready ? (
        <div className="flex flex-col gap-2 rounded-xl border border-line p-3">
          {!state.voice_installed ? (
            <p className="text-muted">Install Voice first (Settings › Engines): it's what finds the faces.</p>
          ) : state.setup.busy ? (
            <p className="text-muted">Setting up known faces (about 80 MB)…</p>
          ) : (
            <>
              <p className="text-muted">Known faces need a small add-on for Voice: OpenCV and two face models, about 80 MB.</p>
              <Button onClick={() => void run("setup", () => request("POST", "/api/faces/setup", { token }))} disabled={busy !== null}>
                Set up known faces
              </Button>
            </>
          )}
          {state.setup.error && <ErrorNote>{`The last try didn't work: ${state.setup.error}`}</ErrorNote>}
        </div>
      ) : (
        <>
          <form
            className="flex flex-wrap items-center gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) void run("new", () => request("POST", "/api/faces", { token, body: { name: name.trim() } }).then(() => setName("")));
            }}
          >
            <input
              aria-label="Their name"
              placeholder="Their name"
              value={name}
              maxLength={60}
              onChange={(e) => setName(e.target.value)}
              className="min-h-9 flex-1 rounded-lg border border-line bg-surface px-3 text-ink"
            />
            <Button type="submit" disabled={!name.trim() || busy !== null}>
              Add someone
            </Button>
          </form>
          {state.faces.length === 0 && <p className="text-muted">Nobody yet.</p>}
          <ul className="flex flex-col gap-3">
            {state.faces.map((f) => (
              <li key={f.id} className="flex flex-col gap-2 rounded-xl border border-line p-3">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="flex-1 font-medium">{f.name}</span>
                  <select
                    aria-label={`${f.name}'s Outpost account`}
                    className={select}
                    value={f.account_id ?? ""}
                    onChange={(e) =>
                      void run(f.id, () => request("PATCH", `/api/faces/${f.id}`, { token, body: { account_id: e.target.value } }))
                    }
                  >
                    <option value="">Doesn't use Outpost</option>
                    {accounts.map((a) => (
                      <option key={a.id} value={a.id}>
                        Uses Outpost as {a.name}
                      </option>
                    ))}
                  </select>
                  <label className="cursor-pointer rounded-lg border border-line px-3 py-1.5 hover:bg-raised">
                    {busy === f.id ? "Adding…" : "Add photos"}
                    <input
                      type="file"
                      accept="image/jpeg,image/png,image/webp"
                      multiple
                      className="hidden"
                      disabled={busy !== null}
                      onChange={(e) => {
                        void addPhotos(f, e.target.files);
                        e.target.value = "";
                      }}
                    />
                  </label>
                  <Button
                    variant="ghost"
                    onClick={() =>
                      void areYouSure(`Forget ${f.name}? Their name and every face of theirs are removed.`).then(
                        (yes) => yes && void run(f.id, () => request("DELETE", `/api/faces/${f.id}`, { token })),
                      )
                    }
                  >
                    Forget
                  </Button>
                </div>
                <div className="flex flex-wrap gap-2">
                  {f.samples.map((s) => (
                    <Thumb
                      key={s.id}
                      token={token}
                      sample={s}
                      onRemove={() => void run(f.id, () => request("DELETE", `/api/faces/samples/${s.id}`, { token }))}
                    />
                  ))}
                </div>
                <p className={`text-xs ${f.samples.length < 5 ? "text-amber" : "text-muted"}`}>
                  {f.samples.length === 0
                    ? "No photos yet."
                    : f.samples.length < 5
                      ? `${f.samples.length} ${f.samples.length === 1 ? "face" : "faces"}: add a few more for reliable matching.`
                      : `${f.samples.length} faces.`}
                </p>
                {notes[f.id] && <p className="whitespace-pre-wrap text-xs text-amber">{notes[f.id]}</p>}
              </li>
            ))}
          </ul>
        </>
      )}
    </div>
  );
}

// Adding a LoRA you already have (trained elsewhere, or on your old Outpost) by uploading
// its .safetensors file, optionally as one of your characters.

import { useState, type FormEvent, type SyntheticEvent } from "react";
import { createPortal } from "react-dom";
import { ApiError, uploadLora, type LoraUploaded } from "../api";
import { Button, ErrorNote, Field } from "./ui";

export function AddLora({
  token,
  onClose,
  onAdded,
}: {
  token: string;
  onClose: () => void;
  onAdded: (added: LoraUploaded) => void;
}) {
  const [file, setFile] = useState<File | null>(null);
  const [isCharacter, setIsCharacter] = useState(true);
  const [name, setName] = useState("");
  const [trigger, setTrigger] = useState("");
  const [subject, setSubject] = useState("");
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [exists, setExists] = useState(false);

  function pick(f: File | null) {
    setFile(f);
    setExists(false);
    setError(null);
    if (f && !name) setName(f.name.replace(/\.safetensors$/i, "").replace(/[_-]+/g, " ").trim());
  }

  async function submit(e: FormEvent | SyntheticEvent, replace = false) {
    // This dialog opens from inside Create's form: keep its submit from reaching that form.
    e.preventDefault();
    e.stopPropagation();
    if (!file) return setError("Pick the LoRA's .safetensors file.");
    if (!/\.safetensors$/i.test(file.name)) return setError("A LoRA is a .safetensors file.");
    if (isCharacter && (!name.trim() || trigger.trim().length < 2))
      return setError("Give the character a name and the trigger word it was trained with.");
    setError(null);
    setExists(false);
    setProgress(0);
    try {
      const added = await uploadLora(
        token,
        file,
        isCharacter ? { replace, character: name.trim(), trigger: trigger.trim(), subject: subject.trim() } : { replace },
        setProgress,
      );
      onAdded(added);
    } catch (err) {
      setProgress(null);
      if (err instanceof ApiError && err.code === "exists") setExists(true);
      setError(err instanceof ApiError ? err.message : "The upload failed.");
    }
  }

  const uploading = progress !== null;
  // Drawn at the top of the page, outside Create's form (a form can't hold another form).
  return createPortal(
    <div role="dialog" aria-modal="true" aria-label="Add a LoRA" className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-6">
      <form onSubmit={(e) => void submit(e)} className="flex w-full max-w-md flex-col gap-4 rounded-2xl border border-line bg-night p-6" noValidate>
        <h2 className="text-lg font-semibold">Add a LoRA</h2>
        <p className="text-sm text-muted">
          Upload a <span className="font-mono">.safetensors</span> LoRA you already have. Like your personas, it's
          yours: other accounts don't see or use it.
        </p>
        <label className="flex flex-col gap-1.5 text-sm text-muted">
          LoRA file
          <input
            type="file"
            accept=".safetensors"
            disabled={uploading}
            onChange={(e) => pick(e.target.files?.[0] ?? null)}
            className="text-sm text-ink file:mr-3 file:rounded-lg file:border file:border-line file:bg-surface file:px-3 file:py-1.5 file:text-ink"
          />
          {file && <span className="text-xs">{(file.size / 1024 / 1024).toFixed(0)} MB</span>}
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={isCharacter} onChange={(e) => setIsCharacter(e.target.checked)} className="accent-ember" disabled={uploading} />
          It's a character (a person, pet or creature), not a style
        </label>
        {isCharacter && (
          <>
            <Field label="Character's name" value={name} onChange={(e) => setName(e.target.value)} maxLength={60} disabled={uploading} hint="What you'll call it in chat, e.g. “draw Kat at the beach”." />
            <Field label="Trigger word" value={trigger} onChange={(e) => setTrigger(e.target.value)} maxLength={40} disabled={uploading} hint="The word it was trained with, e.g. k4tt." />
            <Field label="What it is (optional)" value={subject} onChange={(e) => setSubject(e.target.value)} maxLength={40} disabled={uploading} placeholder="woman, dog, fox character…" />
          </>
        )}
        {uploading && (
          <div className="flex flex-col gap-1" aria-live="polite">
            <div className="h-2 overflow-hidden rounded-full bg-surface">
              <div className="h-full bg-ember transition-[width]" style={{ width: `${Math.round((progress ?? 0) * 100)}%` }} />
            </div>
            <span className="text-xs text-muted">
              {progress !== null && progress >= 1 ? "Checking and saving…" : `Uploading… ${Math.round((progress ?? 0) * 100)}%`}
            </span>
          </div>
        )}
        <ErrorNote>{error}</ErrorNote>
        <div className="flex flex-wrap gap-3">
          {exists ? (
            <Button type="button" onClick={(e) => void submit(e, true)}>
              Replace it
            </Button>
          ) : (
            <Button type="submit" disabled={uploading || !file}>
              {uploading ? "Uploading…" : "Upload"}
            </Button>
          )}
          <Button type="button" variant="ghost" onClick={onClose} disabled={uploading}>
            Cancel
          </Button>
        </div>
      </form>
    </div>,
    document.body,
  );
}

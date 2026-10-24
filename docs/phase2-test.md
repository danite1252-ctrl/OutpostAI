# Phase 2 live test: Create

Everything here was built and unit-tested on this PC, but no picture has been made yet,
because that needs ComfyUI running on the GPU. This is the first real run. About 45
minutes, mostly downloads. Send Claude a screenshot or the error text for anything that
doesn't match.

## 0. Start

```powershell
cd C:\dev\Outpost
npm run dev
```

ComfyUI must be installed (Settings > Engines). It doesn't need to be running: Outpost
starts it when a job needs it.

## 1. Z-Image Turbo (start here: smallest and fastest)

1. Open **Create**. Five workflows are listed across the top; Z-Image Turbo is selected.
2. It says it needs about **20.7 GB** of models. Press **Download**. A bar shows progress
   by file. Leave the page and come back: it keeps going.
3. When it finishes, the form appears. Prompt: `a lighthouse on a cliff at dusk, photo`.
   Press **Generate** (or Ctrl+Enter).
4. Expect, in the Queue: "Starting ComfyUI" (first time, up to a minute or two), then
   "Freeing the GPU", "Loading models", then **Step 1 of 8 … 8 of 8**, then the picture
   appears in the Gallery.
5. Click it: the viewer shows prompt and seed. Try **Use these settings**, then
   Generate again: same seed, same picture.

**Pass:** a picture in under about a minute after models are loaded.

## 2. Your LoRA

1. Copy `C:\dev\ai-toolkit\output\my_first_lora_v1\my_first_lora_v1.safetensors` (or the
   d4nni one) into the LoRA folder shown under the LoRA picker
   (`%LOCALAPPDATA%\Outpost\models\comfyui\loras`). Rename it `d4nni.safetensors`.
2. Press **Refresh** next to the picker, choose it, prompt
   `d4nni, a woman in a red winter coat, snowy city street, photo`.

**Pass:** she's recognisable, as in the ai-toolkit samples.

## 3. Cancel and queue

1. Queue three pictures quickly. The second and third show "1 ahead", "2 ahead".
2. Press **Stop** on the running one and **Cancel** on a queued one. Both disappear; the
   other still runs.

## 4. Qwen-Image-Edit (optional, about 31 GB more; Qwen-Image then needs only 22 GB more, as they share files)

Download, choose a photo (drop it in, paste it or Choose…), ask
`make it night with street lights on`. On 16 GB expect it to be slow (it is bigger than
the card); the first run loads 20 GB.

## 5. Wan 2.2 Image to Video (optional, about 18 GB more)

Choose a picture (a file for now; picking straight from the gallery comes later), `2 seconds`, prompt
`she turns and smiles, slow push-in`. Expect several minutes. The clip appears in the
gallery and plays in the viewer.

## What to send Claude

- Anything that failed: the red error under the job, or a screenshot.
- Time for the Z-Image picture, and for a Wan clip if you tried one.
- Whether chat still answered after a job (Outpost unloads the chat model for each job;
  the next message reloads it, so it is slower once).

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

Already have some model files? Put them in `%LOCALAPPDATA%\Outpost\models\comfyui\<folder>`
with the exact names Create lists, then reopen Create: they won't download again.

## 1. Text to image: Z-Image Turbo (start here)

1. Open **Create**. Tabs across the top: Text to image, Characters, Edit, Video, Animate.
   On Text to image the **Model** row shows Z-Image Turbo in blue (best for this PC) and the
   others in green or orange.
2. It needs about **20.7 GB** of models. Press **Download**. The **Downloads** button at the
   top right shows a ring; open it to see the file, file 1 of 3, speed and time left. Leave
   the page and come back: it keeps going.
3. When it finishes, the form appears. Prompt: `a lighthouse on a cliff at dusk, photo`.
   Press **Generate** (or Ctrl+Enter).
4. Expect, in the Queue: "Starting ComfyUI" (first time, up to a minute or two), then
   "Freeing the GPU", "Loading models", then **Step 1 of 8 … 8 of 8**, then the picture
   appears under Recent.
5. Click it: the viewer shows prompt and seed. Try **Use these settings**, then
   Generate again: same seed, same picture.

**Pass:** a picture in under about a minute after models are loaded.

## 2. Characters: your LoRA

1. Copy your trained `.safetensors` (from ai-toolkit's output folder) into
   `%LOCALAPPDATA%\Outpost\models\comfyui\loras` and name it after the trigger word, e.g.
   `zd4nni.safetensors`.
2. Open the **Characters** tab (it uses Z-Image Turbo's files, so nothing more to
   download), press **Refresh**, choose it, prompt
   `zd4nni, a woman in a red winter coat, snowy city street, photo`.

**Pass:** she's recognisable, as in the ai-toolkit samples.

## 3. Cancel and queue

1. Queue three pictures quickly. The second and third show "1 ahead", "2 ahead".
2. Press **Stop** on the running one and **Cancel** on a queued one. Both disappear; the
   other still runs.

## 4. Edit: Qwen-Image 2.1 Edit (about 17 GB more)

1. Open **Edit**. Qwen-Image 2.1 Edit should be blue. Download it.
2. Picture to edit: press **From gallery…** and pick the lighthouse (or drop in a photo).
   Ask `make it night with the lamp lit`.
3. Optional: add a second picture under **Reference picture 2** (a shirt, say) and ask
   `dress the person in <image1> in the shirt from <image2>`.

**Pass:** an edited picture in about a minute. If it says ComfyUI is too old, send Claude
the message.

## 5. Gallery

1. Open **Gallery** in the side rail. Everything you made is there, newest first; filter
   Pictures / Videos.
2. Open a picture and press **Edit this picture**: Create opens on Edit with it filled in.
3. Open one of yours and press **Share with everyone**: it appears under **Shared**, marked
   "Shared by you". **Stop sharing** takes it out.
4. As the owner you also see a tab per account after Mine and Shared. Make a picture while
   signed in as someone else: they see only Mine and Shared; sign back in as yourself and
   it's under their tab.

## 6. Animate: Wan 2.2 (optional, about 18 GB more)

From a gallery picture press **Animate this picture**, choose `2 seconds`, prompt
`she turns and smiles, slow push-in`. Expect several minutes. The clip appears in the
gallery and plays in the viewer.

## 7. Chat's Context panel

Open **Chat**. On the right: the persona, what it remembers (add a fact, then forget it),
your app keys with a green dot if used in the last 10 minutes, and the `persona:<slug>`
name for Hermes. Hide it with ×; the **Context** button brings it back.

## 8. ComfyUI version check

With ComfyUI running, every workflow should show its normal colour. Any marked grey
"Needs a newer ComfyUI" means your ComfyUI is missing nodes: press **Update ComfyUI** (or
Settings › Engines › Update) and check it goes back to normal afterwards.

## What to send Claude

- Anything that failed: the red error under the job, or a screenshot.
- Time for the Z-Image picture, the 2.1 edit, and a Wan clip if you tried one.
- Whether the Downloads speed and time left looked right.
- Whether chat still answered after a job (Outpost unloads the chat model for each job;
  the next message reloads it, so it is slower once).

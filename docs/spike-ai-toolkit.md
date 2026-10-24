# Spike: can ai-toolkit train image LoRAs on a 16 GB card?

The last Phase 1 item. Phase 4 (Training Studio) builds on ai-toolkit, so check now
that it trains **Z-Image Turbo** (the default base) on this PC
(RTX 4080 SUPER, 16 GB) before anything is built around it. It's a manual test,
about 3 hours, mostly waiting. (Video/Wan training is out of scope: image LoRAs only.) Nothing here changes Outpost.

## 1. Install (about 15 minutes, a few GB)

Close Outpost first. Then, in PowerShell:

```powershell
cd C:\dev\Outpost
powershell -ExecutionPolicy Bypass -File .\scripts\spike\install-ai-toolkit.ps1
```

It installs to `C:\dev\ai-toolkit`, using the uv that Outpost already has, so no
system Python is touched. The last line should say `cuda True` and name your card.

## 2. Make a small dataset (about 10 minutes)

1. Put 15 to 25 photos of one subject (a pet, a person who agreed, an object) in a
   folder, e.g. `C:\datasets\rex`. Varied angles and backgrounds; at least 1024 px.
2. Caption them with Outpost's vision model (Outpost running, a vision model such as
   Ornith 1.5 9B downloaded, an app key from Settings > App keys):

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\spike\caption-folder.ps1 -Folder "C:\datasets\rex" -Trigger "rexdog" -Key "op_YOUR_KEY" -Model "ornith-1.5:9b"
```

   Each image gets a `.txt` beside it, starting with the trigger word. Skim a few
   and fix anything wrong. Then **close Outpost** so the trainer has the whole GPU.

## 3. Train Z-Image Turbo (pass/fail test 1)

```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\spike\start-ai-toolkit.ps1
```

Open http://localhost:8675, add the dataset folder, then make a new job:

- Model: **Z-Image Turbo** (it uses the training adapter it suggests).
- Turn on **Low VRAM** and the quantisation it offers for the transformer.
- **Trigger Word**: your word, e.g. `rexdog`.
- **Target > Linear Rank**: 16. **Training > Steps**: 2000. Leave **Learning Rate**
  as it is; **Batch Size** 1.
- **Dataset > Resolutions**: tick only 1024 (768 if it runs out of memory).
- **Sample**: Sample Every 250; replace the prompts with one or two that use the
  trigger word, e.g. "rexdog sitting on a beach".

Watch GPU memory on ai-toolkit's own dashboard (it shows GPU, CPU and RAM live) and
note the highest GPU memory you see.

**Pass** if it finishes without an out-of-memory error and the later samples clearly
show the subject.

## 4. Write down

| Test | Settings changed from the above | Peak VRAM | Seconds per step | Total time | Result |
| --- | --- | --- | --- | --- | --- |
| Z-Image Turbo | 1024, rank 16, 2000 steps | 15.4 GB | ~4.8 | ~2 h 40 m | |

Send the table (and any error) to Claude: it sets Training Studio's defaults and the
VRAM rules in the model catalogue.

## Clean up (optional)

Delete `C:\dev\ai-toolkit` and the dataset folder. Model downloads sit in your
Hugging Face cache (`%USERPROFILE%\.cache\huggingface`), which can be large.

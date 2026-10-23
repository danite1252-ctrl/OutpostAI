# Spike: can ai-toolkit train on a 16 GB card?

The last Phase 1 item. Phase 4 (Training Studio) builds on ai-toolkit, so check now
that it trains **Z-Image Turbo** (the default base) and **Wan 2.2** on this PC
(RTX 4080 SUPER, 16 GB) before anything is built around it. It's a manual test,
about 2 to 3 hours mostly waiting. Nothing here changes Outpost.

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
- Resolution 1024 (drop to 768 if it runs out of memory), LoRA rank 16,
  about 2,000 steps, the default learning rate, batch size 1.
- A sample prompt that uses the trigger word, e.g. "rexdog sitting on a beach".

In a second PowerShell window, watch memory while it runs:

```powershell
nvidia-smi --query-gpu=memory.used,memory.total,utilization.gpu --format=csv -l 10
```

**Pass** if it finishes without an out-of-memory error and the later samples clearly
show the subject.

## 4. Train Wan 2.2 (pass/fail test 2)

Same dataset (images only is fine for a first test):

- Try **Wan 2.2 TI2V 5B** first: Low VRAM on, resolution 512, rank 16, 1,000 steps.
- If that works, try **Wan 2.2 T2V 14B** with Low VRAM and 4-bit quantisation,
  resolution 512, for 300 steps only, to see whether it runs at all.

**Pass** if TI2V 5B finishes. The 14B result decides whether Training Studio offers
14B on 16 GB cards or only on 24 GB+.

## 5. Write down

| Test | Settings changed from the above | Peak VRAM | Seconds per step | Total time | Result |
| --- | --- | --- | --- | --- | --- |
| Z-Image Turbo | | | | | |
| Wan 2.2 TI2V 5B | | | | | |
| Wan 2.2 T2V 14B | | | | | |

Send the table (and any error) to Claude: it sets Training Studio's defaults and the
VRAM rules in the model catalogue.

## Clean up (optional)

Delete `C:\dev\ai-toolkit` and the dataset folder. Model downloads sit in your
Hugging Face cache (`%USERPROFILE%\.cache\huggingface`), which can be large.

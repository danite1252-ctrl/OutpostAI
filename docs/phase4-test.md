# Phase 4 live test: Training Studio

The gate: **a trained character is used by name in chat.**

Needs: ai-toolkit in `C:\dev\ai-toolkit` (from the spike), Z-Image Turbo downloaded in
Create › Characters, a vision model in Models (the eye badge), and 15 to 30 pictures.

## Overnight run (Claude starts this)

The test runner (v3) has a `train` job. With the Outpost desktop app **closed**, it builds
the gateway and starts it with:

```
outpost-gateway --queue-training C:\dev\dataset\kat --train-name Kat --train-trigger zk4t --train-steps 2000
```

That makes a "Kat" dataset for the owner, copies the 20 pictures, captions them with the
first vision model, and queues the training. Its log is `target\overnight\train-gateway.txt`.
About 2.5 hours on a 16 GB card. In the morning, stop it (`train-stop`), then open the
desktop app as normal: it finds everything.

## 1. Morning: pick a checkpoint (desktop)

1. Open Outpost, **Train** › **Kat**. The trigger line shows ✓ for `zk4t`.
2. Pictures: 20, each with a caption. Fix any that are wrong (click out to save).
3. The finished training shows rows of test pictures: "Before", then steps 250 to 2000.
4. Press **Use this one** on the row that looks most like Kat (often 1250 to 1750).

**Pass:** the row shows "✓ In use", and Kat appears under Characters on the left.

## 2. Create › Characters

Pick **Kat (zk4t)** from "My characters" in the LoRA list, prompt `zk4t, reading in a café, photo`.

**Pass:** it looks like Kat.

## 3. Chat: the gate

1. **Desktop:** in any chat, send `draw Kat at the beach at sunset`.
2. **Phone:** send `/draw zk4t in a red winter coat in the snow`.

**Pass:** the reply says "Drawing Kat with Z-Image Turbo", shows its place and progress,
then the picture, which looks like Kat. Ordinary messages still get normal replies.

## 4. Trigger words and the queue

1. Train › **New character** › name `Danni`. The suggested trigger is `zd4nni` (✓).
2. Change the trigger to `danni`: a yellow warning explains why, with **Use zd4nni**.
3. Optional (takes the GPU): add 5+ pictures, **Start training**, watch the Queue button
   ("Training a character", step and time left), then **Stop**. Chat says it's paused
   while training runs, and works again after.

## 5. Test prompts follow the pictures

On any dataset with captions, open **Start training** › "Test pictures". The three
prompts start with the trigger word and use the captions' subject and style (e.g.
"zk4t, woman, …, realistic photography"; a dog dataset says "dog", a cartoon one keeps
"cartoon style"). Edit one, and "Back to the suggested prompts" appears. After a training,
the prompts are listed above its checkpoint rows, and hovering a test picture shows its prompt.

## 6. Deleting chat messages

1. **Desktop:** hover a message you sent › **Delete** › OK. It and its reply go. Hover a
   reply › **Delete** removes just the reply.
2. **Phone:** long-press a message › **Delete**.
3. Tell a persona with memory on something new ("I keep bees"), wait for it to show in
   the chat's Context panel, then delete that message.

**Pass:** the message goes, and so does the memory it taught (Context panel / Memory).
Memories learned before this change only know their chat, so they stay until removed
from the Memory list.

## 7. Character defaults

Create › Characters: pick Kat. The prompt hint names her and her trigger word. Steps
(More settings) is 16 and LoRA strength 1.1. `draw Kat …` in chat uses the same.

## 8. A character trained elsewhere

Train › Characters › **Add from a LoRA file…**, pick `char_zzz_kat_zimage`, give it a
name, its trigger word, "woman" and "photo", then **Add character**. It shows in
Create › Characters and works with "draw <name> …" in chat. Removing it leaves the file.
`zd4nni` fills in its own trigger word; the Qwen Lightning LoRA can't be picked.

## 9. Several pictures from one message

- `draw 4 pictures of Kat at the beach` (or `… x4`): four cards, four different pictures.
- `draw Kat at the beach; in the snow; reading in a café`: three scenes, all of Kat.

**Pass:** one reply listing the prompts, with a card per picture that fills in. At most 8.

## What to send Claude

Which checkpoint you picked, how the chat pictures look, and anything that failed
(with `target\overnight\train-gateway.txt` if the overnight run went wrong).

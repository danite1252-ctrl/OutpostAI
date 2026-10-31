# Outpost user guide

Outpost turns one Windows PC (or a Linux server) into a private AI server for your household
or small team: chat with personas that remember things and read your documents, make and edit
pictures, short videos and 3D models (ready for your 3D printer), train characters from your
own photos, and talk out loud, on your own graphics card. Phones and
other apps use the same PC. Nothing goes to a cloud AI service.

- [What you need](#what-you-need)
- [Install](#install)
- [First start](#first-start)
- [Chat and personas](#chat-and-personas)
- [Create, Gallery and the queue](#create-gallery-and-the-queue)
- [Train a character](#train-a-character)
- [Voice](#voice)
- [The phone app](#the-phone-app)
- [Other apps: API, MCP and webhooks](#other-apps-api-mcp-and-webhooks)
- [Updates](#updates)
- [Privacy: what leaves your PC](#privacy-what-leaves-your-pc)
- [Your data, backups and uninstalling](#your-data-backups-and-uninstalling)
- [Troubleshooting](#troubleshooting)

## What you need

| | Minimum | Better |
| --- | --- | --- |
| Windows | 10 or 11, 64-bit | |
| Graphics card | NVIDIA with 8 GB (chat and pictures) | NVIDIA with 16 GB or more (video, character training) |
| Memory | 16 GB | 32 GB |
| Disk | 30 GB free | 100 GB or more: models are large |

Without an NVIDIA card, chat still works (more slowly) and pictures run on the processor,
which is very slow. Voice runs on the processor either way.

## Install

1. Download **`Outpost_<version>_x64-setup.exe`** from
   [the releases page](https://github.com/danite1252-ctrl/OutpostAI/releases).
2. Open it. Windows may say **"Windows protected your PC"** because the installer isn't
   signed with a paid certificate yet: press **More info**, then **Run anyway**.
3. It installs for your Windows user only (no administrator needed) and adds Outpost to the
   Start menu.

## First start

1. **Make the owner account**: your name and a PIN (or password). The owner manages
   accounts, models and connections.
2. **Install the engines** Outpost offers. They download once and are kept up to date by
   Outpost; you can do this later in **Settings › Engines**:
   - **Ollama** runs chat models.
   - **ComfyUI** makes pictures and videos (about 4 GB with PyTorch).
   - **Voice** hears you (Whisper) and speaks replies (Kokoro), about 1 GB.
3. **Get a chat model** in **Models**. Outpost suggests ones that fit your graphics card.
4. Add other people in **Settings › Accounts**. Everyone picks their name on the
   **Who's using Outpost?** screen and signs in with their own PIN; their chats, memories
   and pictures are their own.

**Closing the window** keeps Outpost running in the system tray (by the clock), so phones,
apps and the picture queue keep working. Click the tray icon to open it again; right-click
it and choose **Quit Outpost** to stop everything. Turn this off in **Settings › This window**.

The bar at the top shows what's running: the gateway (Outpost itself), Ollama, ComfyUI,
Whisper and Kokoro.

## Chat and personas

- **Chat**: start a chat with a persona. Replies are formatted (lists, tables, code with a
  Copy button). Attach up to four pictures for a model that can see them.
- **Rename a chat**: the pencil beside it in the list (or double-click it); on the phone,
  long-press it. A new chat is named after its first message until you do.
- **Delete a message** (long-press on the phone): your message takes its reply with it, and
  anything the persona learned from it is forgotten.
- **Personas** (side bar): name, colour, model, instructions, voice, and **memory**. With
  learning on, the persona notes lasting facts you mention and recalls the relevant ones
  later. You can see, edit and forget every memory.
- **Draw from chat**: `/draw a lighthouse at dusk`, `draw Kat at the beach` (with a
  character's name), `4 pictures of Kat at the beach`, or several scenes separated by `;`.
  The picture appears in the chat and the Gallery.

## Create, Gallery and the queue

- **Create** has a tab per task: **Picture**, **Character**, **Edit**, **Video** and
  **Animate**. Outpost picks workflows that suit your graphics card and downloads their
  model files the first time (the **Downloads** button shows progress).
- Jobs wait their turn in the **queue** (top bar), one at a time on the graphics card. Chat
  pauses image work automatically and the other way round, so they don't fight over memory.
- **3D**: a 3D model from a picture of one thing (its background is removed for you).
  Hunyuan3D makes a plain shape on an 8 GB card; TRELLIS.2 makes a coloured one and needs
  about 16 GB (a lent PC can make it). **Ready for 3D printing** (on by default) also saves a
  `.3mf` for Bambu Studio and other slicers: standing on the bed at the height you chose, in
  up to 4 colours (load the filaments in the order Outpost shows), with an optional flat
  bottom or back. Turn it off for games and 3D apps (`.glb` only).
- **Gallery**: everything you made, with the prompt and settings. New things appear as
  they're made. Search it by what's in the pictures, share with the household, edit,
  animate, upscale, remove a background, make a 3D model, or delete. 3D models turn round
  with the mouse (or a finger on the phone).
- **Names**: give a 3D model a name when you make it (Create › 3D › Name), or later with
  **Name it** / **Rename** in the gallery (any picture or video too). Saving uses it for the
  file (`Sheep.3mf`), and gallery search finds it.
- **Saving**: on the PC, **Save** puts the file in your **Downloads** folder and says so,
  with **Show in folder**. On the phone it goes to Pictures, Movies or Download (an Outpost
  folder in each), then **Share it…** sends it on (to Bambu Handy for cloud printing, say).
- **Notifications**: when a long job (3D, video, training, or anything over a minute)
  finishes or fails, Windows and the phone tell you, unless you're looking at Outpost. Switch
  it off in **Settings › Updates, notifications and this window** (PC) or **You** (phone).

## Train a character

**Train** (side bar) makes a character from 15–40 photos of one person, pet or style:

1. Make a dataset and add the photos (or import a folder). Outpost writes a caption for
   each with a vision model; check them. Captions describe the scene, not the person.
2. Pick a **trigger word** (Outpost checks it isn't a real word) and press **Train**. It
   runs on the graphics card for an hour or more; test pictures appear as it goes.
3. **Use** the best step. The character then works by name everywhere: Create's
   Character tab, `/draw Kat at the beach` in chat, the phone, and other apps.

You can also import a LoRA you trained elsewhere as a character.

Training needs about 16 GB of video memory. With a server whose card is smaller, a PC that
lends its graphics card can train instead: on that PC, **Settings › Server › Lend this PC's
graphics card**, then **Install ai-toolkit here**. Its checkpoints come back to the server as
they're saved. Pausing lending (for gaming) stops a training and keeps the checkpoints so far.

## Voice

Install **Voice** in **Settings › Engines**, then:

- **Give a persona a voice** in Personas (54 voices; **Hear it** to try one) and tick
  **Read replies aloud** if you want every reply spoken.
- **Desktop chat**: the microphone button turns what you say into text in the box;
  the play button on a reply reads it out.
- **Hands-free** (desktop) and **Talk** (phone): a spoken conversation, like a call. Just
  talk, then pause; the persona answers out loud and listens again. Talk over a reply to
  cut in (a cough or a door only pauses it for a moment). "Draw Kat at the beach" works too.

Speech is recognised by Whisper on your PC. The phone's **Mic** button uses the phone's own
recogniser (on the phone where it can, otherwise the phone's speech service).

## The phone app

1. Install **`Outpost-<version>.apk`** from
   [the releases page](https://github.com/danite1252-ctrl/OutpostAI/releases) on the phone
   (Android asks you to allow installs from your browser or file manager).
2. On the PC: **Connections › Allow other devices on my network** (or **Allow my Tailscale
   devices**), restart Outpost, then press **Pair a phone**.
3. On the phone: scan the code, check the words match the PC's, and sign in.

The phone talks only to your PC, over an encrypted connection checked against the PC's own
certificate. Away from home it works over **Tailscale** (Connections › Allow my Tailscale devices). It has
chat (with voice), Create, the Gallery and the queue.

**Phone app updates** download by themselves (about 90 MB, from the GitHub release) and
install when you press **Restart to update**. The first time, Android asks you to allow
Outpost to install apps and to confirm. After that, updates install by themselves the next
time you leave the app. Switch this off in **You › Updates**. Android only installs an
update signed with the same key as the app you have, and you stay paired.

## Other apps: API, MCP and webhooks

The **API** page (side bar) has everything with copy-ready examples:

- **OpenAI-compatible** endpoints (`/v1/chat/completions`, `/v1/audio/speech`,
  `/v1/audio/transcriptions`) for apps like Hermes. Make an **app key** in Connections.
- **MCP** for apps like Claude Desktop: chat with a persona, draw (characters by name),
  recent pictures, and memory.
- **Webhooks** (Connections): a signed message to your own URL when a picture, training or
  new character is done.

## Updates

Once a day Outpost asks GitHub whether there's a newer version. When there is, a bar at the
top says so: **What's new** shows the notes, and **Update and restart** installs it (the
download is checked against Outpost's signing key first). Nothing installs without you
pressing the button.

Switch the daily check off, or check now, in **Settings › Updates**.

## Privacy: what leaves your PC

Your chats, memories, pictures, recordings and characters stay on your PC. Outpost
contacts the internet only for:

| When | Where | What |
| --- | --- | --- |
| Once a day, unless switched off | github.com | Asks for the latest version number. Nothing about you is sent. |
| The phone app opens (at most every 20 minutes), unless switched off | github.com | The phone asks for the latest version number, and downloads a newer phone app |
| You install or update an engine | github.com, pypi.org, download.pytorch.org, huggingface.co (voice) | The engine's files |
| You download a model | ollama.com, huggingface.co | The model's files |
| You add a webhook | Your own URL | The events you chose |
| Tailscale is on | Tailscale's network | Encrypted traffic between your own devices |

The phone's **Mic** button may use the phone's speech service when the phone can't
recognise speech by itself; hands-free on the phone always uses Whisper on your PC.

## Your data, backups and uninstalling

Everything lives in **`%LOCALAPPDATA%\Outpost`**:

| Folder | What |
| --- | --- |
| `outpost.db` | Accounts, chats, personas, memories, settings |
| `files` | Chat pictures, gallery, datasets and characters |
| `models` | Picture, video and voice models |
| `engines` | Ollama, ComfyUI, the voice service and their Python |
| `logs`, `crash-reports` | Diagnostics (see below) |

**Back up** by closing Outpost and copying `outpost.db` and `files` (and `models`, if you'd
rather not download them again).

The app itself (`Outpost.exe`, `outpost-gateway.exe`) is installed in the same folder.

**Uninstalling** (Windows Settings › Apps) removes the app's own files but keeps your data,
so a reinstall picks up where you left off. Delete the folder too to remove everything.

## Troubleshooting

- **Settings › Diagnostics** lists crash reports and shows the recent log. If Outpost had
  a problem last time, a note at the top says so. Reports never leave your PC; use **Copy**
  to share one when asking for help.
- **An engine won't start**: Settings › Engines shows its log. **Reinstall** keeps your models.
- **The phone can't connect**: check the PC is on, Outpost is open, network access is on,
  and the phone is on the same Wi-Fi (or Tailscale is on). Windows Firewall must allow
  Outpost on **private** networks.
- **Hands-free doesn't hear you on the phone**: turn on **You › Troubleshooting › Voice
  check line** to see the microphone and room levels while you talk.
- **Pictures are slow or fail with "out of memory"**: close other programs using the
  graphics card, or pick a lighter workflow in Create.

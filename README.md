# Outpost

Your private AI outpost: a Windows (then Linux) app that turns one PC into a local AI
server for a household or small team. Chat, personas, images, video and training, all
on your own GPU, all reachable over one API so phones and apps like Hermes can plug in.

Everything runs locally. No cloud models, accounts or services. MIT licensed.

> Status: **phase 2, Create (in testing).** Phase 1 (chat) is complete: accounts with PINs,
> the local gateway and API spec, managed Ollama and ComfyUI, the model catalogue, personas,
> chat with formatted replies and images, memory that learns, app keys for Hermes and network
> access. Phase 2 adds Create: image, edit, character and video workflows picked to suit your
> graphics card, a GPU queue, a gallery and a downloads list.

## Layout

| Path | What it is |
| --- | --- |
| `crates/outpost-core` | Database, accounts, PIN hashing, sessions, audit log |
| `crates/outpost-engines` | Installs, starts and supervises Ollama and ComfyUI |
| `crates/outpost-gateway` | The HTTP API on port 7420 (library + `outpost-gateway` binary) |
| `apps/desktop` | Tauri 2 + React desktop app; starts the gateway if none is running |

## Prerequisites (Windows)

1. **Rust** (stable) from <https://rustup.rs>.
2. **Visual Studio Build Tools** with "Desktop development with C++" (Rust needs the MSVC linker).
3. **Node.js 22** from <https://nodejs.org>.
4. **WebView2**: already on Windows 10/11.

On Linux you also need the Tauri system packages; see
<https://tauri.app/start/prerequisites/>.

## Run it

```powershell
npm install          # once
npm test             # Rust tests for core + gateway
npm run dev          # opens the desktop app with hot reload
```

First launch asks you to create the **owner** account (name + PIN or password), then
offers to install the engines. After that the app opens on "Who's using Outpost?".

## Engines

Outpost installs pinned versions into `<data folder>\engines` and runs them hidden:

| Engine | Version | How | Port |
| --- | --- | --- | --- |
| Ollama | v0.40.9 | Official portable archive, checksum verified | 11434 |
| ComfyUI | v0.42.5 | Source release + Python 3.13 via uv 0.13.5, PyTorch cu130 (NVIDIA) or CPU | 8188 |

Models are kept in `<data folder>\models`, so reinstalling an engine never deletes them.
Ollama starts with a 64K-token context window (flash attention, 8-bit KV cache), which
agents like Hermes need.
If you already run your own Ollama or ComfyUI on those ports, Outpost uses it instead
and shows it as "your own copy". Engines stop when the app closes. Versions live in
`crates/outpost-engines/src/versions.rs`.

Run only the API, without the window:

```powershell
npm run gateway                      # or: cargo run -p outpost-gateway -- --help
curl http://127.0.0.1:7420/api/health
npm run openapi > openapi.json       # the API spec
```

Data lives in `%LOCALAPPDATA%\Outpost` on Windows and `~/.local/share/Outpost` on Linux
(change it with `--data-dir` or `OUTPOST_DATA_DIR`).

## Connect Hermes (or any OpenAI-compatible app)

1. Settings → Accounts: add an account with role **Service**, named e.g. "Hermes".
2. Settings → App keys: pick that account, create a key, copy it.
3. In the app: base URL `http://127.0.0.1:7420/v1`, API key = the key, model `persona:assistant`.

The key sees only that account's personas and memory. Revoke it any time in Settings.

## Chat features

- **Formatted replies**: headings, lists, tables, quotes and code blocks with a Copy
  button, rendered by Outpost's own Markdown renderer (no HTML from the model is ever run).
- **Images**: attach up to 4 (PNG, JPEG, WebP, GIF; paste, drop or pick) for a model with
  vision, e.g. Ornith 1.5 9B. Large photos are scaled to 1600 px before sending. Images
  are kept per account in `<data folder>\files\<account>\images` and deleted with their chat.
- **Memory**: facts you add are always used. With learning on (Personas → Memory), after
  each reply the chat's model notes lasting facts you mention; the relevant ones are
  recalled in later chats. Download **Qwen3 Embedding 0.6B** for recall by meaning;
  without it the newest facts are used. Review, forget or clear them any time.

### Hermes on another machine

1. Settings → Network → **Allow other devices on my network**, then close and reopen Outpost.
   It now listens on `0.0.0.0:7420`. Windows asks about the firewall: allow **Private networks**.
2. Settings → App keys shows the address to use, e.g. `http://192.168.1.20:7420/v1`.
3. Point Hermes at that base URL with the app key, and set `context_length: 65536` for
   the model in Hermes' `config.yaml`.

From another machine only app keys work. PIN sign-in, the account list, first-run setup and
the network switch itself stay on this PC, and desktop sign-ins are refused if they turn up
from the network. Ollama and ComfyUI stay on `127.0.0.1`; Hermes reaches models through
Outpost. Traffic is plain HTTP, so use it on a network you trust (Tailscale comes in phase 3).

If the firewall prompt never appeared, add the rule yourself (admin PowerShell):

```powershell
New-NetFirewallRule -DisplayName "Outpost 7420" -Direction Inbound -Protocol TCP -LocalPort 7420 -Action Allow -Profile Private
```

Running only the gateway: `npm run gateway -- --bind 0.0.0.0:7420` (or set `OUTPOST_BIND`).
Without `--bind` it follows the saved setting.

## Create

Create runs **workflow packs** in ComfyUI. Each pack is a ComfyUI graph (API format) plus a
`pack.json` describing its inputs and model files (`crates/outpost-engines/packs/<id>/`), so
the desktop app, the phone and other apps all drive it the same way.

Create has a tab per **task**. Each task can have several packs; Outpost picks the
best-ranked one that fits the graphics card (or, if none fits, the smallest that runs), and
the Model row lets you switch, coloured like the Models page: blue best for this PC, green
fits, orange runs slowly (asks before downloading), red too big.

| Task | Pack | Models to download | Video memory |
| --- | --- | --- | --- |
| Text to image | Z-Image Turbo: photoreal, 8 steps | about 21 GB | ~14 GB |
| Text to image | Qwen-Image 2.1 Turbo: posters and lettering, 8 steps, up to 2048 px | about 17 GB | ~11 GB |
| Text to image | Qwen-Image: long prompts and text; Fast (Lightning) or Quality | about 32 GB | ~20 GB |
| Characters | Z-Image Turbo with your character LoRA (required) | Z-Image Turbo's files | ~14 GB |
| Edit | Qwen-Image 2.1 Edit: 8 steps, up to two reference pictures (`<image2>`, `<image3>`) | about 17 GB (shared with 2.1 Turbo) | ~11 GB |
| Edit | Qwen-Image-Edit 2509: Fast (Lightning) or Quality | about 31 GB | ~20 GB |
| Video | Wan 2.2 (5B): 2 to 5 second 720p clips | about 18 GB | ~12 GB |
| Animate | Wan 2.2 (5B), starting from your picture | (same files) | ~12 GB |

Outpost reads ComfyUI's version and node list (`/system_stats`, `/object_info`) and greys out
any workflow that needs nodes your ComfyUI lacks ("Needs a newer ComfyUI"), offering **Update
ComfyUI** when Outpost installed it (or saying how, for one you installed yourself). It never
picks such a workflow, and refuses jobs for it with `comfyui_too_old`. Qwen-Image 2.1 needs a
recent ComfyUI; Outpost installs 0.42.5, which has it. Its licence is Qwen's research licence; check it before
commercial use.

- **Models** download from Create, once, into `<data folder>\models\comfyui` (shared between
  packs, resumable, disk space checked first). ComfyUI is pointed there, so reinstalling it
  keeps them.
- **The queue** runs one job at a time, oldest first, for everyone. Before a job Outpost
  unloads Ollama's chat model to free the GPU (chat reloads it on the next message), and
  starts ComfyUI if it isn't running. Each account can have 20 jobs waiting.
- **The gallery** keeps results per account in `<data folder>\files\<account>\gallery`. It
  has its own screen with **Mine** and **Shared** tabs (anyone can put one of theirs in
  the shared gallery with "Share with everyone", and take it out again); only the owner also
  gets a tab per account, to see everyone's. From a picture you can repeat it ("Use these settings"), edit it or
  animate it, and image inputs can be picked straight from it.
- **Queue button** (top right, from any screen, for everyone): how many jobs are in line for
  the graphics card, a ring that fills as the running one progresses, and the line in order.
  Your jobs show what they are, their step, time left and Stop/Cancel; other people's show
  only whose they are ("Sam is creating · 60% · 1 min left"), never what they're making.
- **Downloads**: the button at the top right lists every download (chat models and Create
  files) with the file it's on, speed and time left.
- **LoRAs**: put `.safetensors` files in `<data folder>\models\comfyui\loras` and pick them in
  the form (Characters, or a style LoRA in Z-Image Turbo and Qwen-Image). Start the prompt
  with the LoRA's trigger word.

## Phones and connections (phase 3a)

**Connections** in the side bar holds paired phones, app keys and network access.

- **Encrypted port.** With "Allow other devices on my network" on, Outpost also serves
  **port 7421 over TLS**, with a certificate it makes itself (`<data folder>\tls`). Phones
  must use it; their tokens are refused over plain HTTP. Hermes and other app keys keep
  using port 7420 as before.
- **Pairing.** "Pair a phone" shows a one-time 6-digit code (10 minutes, five wrong guesses
  and it's dead), a QR code and three check words made from the certificate. The phone pins
  the certificate (from the QR, or after the words match), then picks an account and enters
  its PIN, and stays signed in.
- **Found on the network.** Outpost announces itself over mDNS as `_outpost._tcp` (name,
  version, ports, hardware tier; nothing about accounts).
- **Phones confirm risky actions** (deleting models, making app keys, changing accounts) by
  sending the PIN again in an `X-Outpost-Pin` header.
- **Queue order.** Jobs from the desktop go first, then phones, then app keys.
- **Away from home (Tailscale, phase 3c).** "Allow my Tailscale devices" makes both ports
  answer on this PC's Tailscale address (100.x, and its MagicDNS name), even with home
  network access off. Pairing QR codes list the home address first, then Tailscale; the
  phone tries them in order and remembers the one that answered, and already-paired
  phones learn new addresses from `/api/connection`. Nothing is opened to the internet.
- **No Cloudflare tunnel.** A tunnel would swap in Cloudflare's certificate (breaking the
  pinning that proves it's your PC) and make outside requests look local. Tailscale covers
  away-from-home use without either problem.
- **App keys can't manage Outpost.** Making app keys, changing accounts, deleting models
  and installing engines or workflows need the desktop app or a phone (with its PIN); an
  app key gets 403, since it may travel over plain HTTP.
- **Wrong PINs** are counted before they're checked, so a burst of guesses still locks the
  account after five; a wrong PIN during pairing also counts against the code.
- **Caution:** Outpost trusts requests from 127.0.0.1 as "on this PC". Don't put a proxy or
  port forward (SSH tunnel, reverse proxy) in front of port 7420 on the Outpost PC; it would
  make remote requests look local. Phase 3c's Tailscale and Cloudflare support is designed
  around this.

## Phone app (phase 3b)

`apps/mobile` is the Android app (Expo SDK 54, React Native). It pairs with the QR code
or by picking the Outpost from the list found on the Wi-Fi, opens with the phone's own
fingerprint or face, and has Chat (streamed replies, photos), Create (every task and
workflow), Gallery (mine and shared, videos, the owner's view of others) and the Queue.
Pictures from the Outpost gallery can be attached in Chat and used in Create, on the phone
and the desktop.

It talks only to the encrypted port, through its own small native module
(`apps/mobile/modules/outpost-net`, Kotlin): OkHttp with a trust manager that accepts
exactly the certificate pinned at pairing, plus Android's network service discovery for
`_outpost._tcp`. The pairing token lives in the Android Keystore (expo-secure-store).

Build a test APK (Android Studio's SDK and Java are used):

```powershell
cd C:\dev\Outpost\apps\mobile
npm install
npx expo prebuild --platform android
cd android; .\gradlew.bat assembleDebug
```

Then `adb install -r app\build\outputs\apk\debug\app-debug.apk`, or copy the APK to
the phone.

## Train (phase 4)

Training Studio teaches Outpost a character from 15 to 30 pictures, as a LoRA for
Z-Image Turbo. It uses [ai-toolkit](https://github.com/ostris/ai-toolkit) (Ostris, MIT),
installed separately: by default in `C:\dev\ai-toolkit` (set another folder in Train).
`scripts/spike/install-ai-toolkit.ps1` installs it.

1. **New character**, then add pictures (pick, drop, or copy a folder on this PC).
2. **Trigger word**: the made-up word that calls the character up. Outpost warns about
   real words and names (`kat`, `danni`, `rexdog`) and suggests a safer one (`zk4t`).
3. **Captions**: a vision model from Models (the eye badge) writes one per picture;
   edit any. They describe only what changes between pictures (clothes, place, pose,
   light), never the face, hair, skin or build: whatever a caption names, the model ties
   to those words instead of to the trigger word. The trigger is put in front when training.
4. **Start training**: it waits in the same GPU queue as pictures (shown in the Queue
   button) and takes 2 to 3 hours for 2000 steps on a 16 GB card. ComfyUI is stopped and
   chat pauses while it runs, so nothing runs out of memory.
5. Every 250 steps it saves a checkpoint and draws the same test scenes. The test
   prompts are made from the captions (a dog stays a dog, a cartoon stays a cartoon)
   and can be edited before starting. Compare
   them and press **Use this one**: the LoRA is copied into the LoRA folder as
   `<trigger>.safetensors` and the character shows by name in Create › Characters.
6. **In chat**: `/draw <anything>`, or "draw Kat at the beach" naming a character. The
   reply is a card that fills in with the picture (desktop and phone). Several at once:
   `draw 4 pictures of Kat at the beach`, or scenes split by `;` or new lines (up to 8).

A Z-Image LoRA trained elsewhere becomes a character too: copy it into the LoRA folder,
then Train › Characters › **Add from a LoRA file…** (Outpost fills in the trigger word
when the file recorded it, as ai-toolkit's do).

Runs live in `<data>/training/` (datasets, and `runs/<id>/` with `config.json`,
`train.log`, checkpoints and samples). For an unattended test,
`outpost-gateway --queue-training <folder> --train-name Kat --train-trigger zk4t` makes
the dataset for the owner, captions it, queues the training and keeps serving.
`docs/phase4-test.md` is the live test.

The earlier spike (`docs/spike-ai-toolkit.md`, `scripts/spike/`) checked ai-toolkit on a
16 GB card before this was built.

## Other apps: API page, MCP and webhooks (phase 5a)

- **API page** (desktop › API): where apps connect, setup for Hermes / OpenAI-style apps,
  MCP setup for Claude Desktop, and every endpoint with a curl line.
- **MCP server** at `/mcp` (Streamable HTTP, app key as bearer). Tools: `chat`,
  `list_personas`, `draw` (characters by name, several pictures, returns the images),
  `job_status`, `list_characters`, `recent_pictures`, `get_picture`, `remember`. For apps
  that start local programs (Claude Desktop): `outpost-gateway --mcp-stdio` with
  `OUTPOST_KEY` set bridges stdio to the running Outpost.
- **Webhooks** (Connections › Webhooks): a signed JSON POST to your URL on
  `job.finished`, `training.finished` and `character.created`. Check
  `X-Outpost-Signature: sha256=<HMAC-SHA256 of the body with the secret>`; three tries,
  last result shown. `scripts/webhooks/listen.ps1` is a test receiver.

`docs/phase5a-test.md` is the live test.

## API

| Method | Path | Auth | What |
| --- | --- | --- | --- |
| GET | `/api/health` | none | Up? Version, PC name, whether setup is needed |
| POST | `/api/setup/owner` | none, this PC only | Create the first owner and sign in |
| GET | `/api/accounts/directory` | none, this PC only | Accounts for the sign-in screen |
| POST | `/api/auth/sign-in` | none, this PC only | PIN or password → bearer token |
| POST | `/api/auth/sign-out` | bearer | End this session |
| GET | `/api/me` | bearer | Who am I |
| PUT | `/api/me/secret` | bearer | Change my PIN (signs out everywhere) |
| GET, POST | `/api/accounts` | owner | List / add accounts |
| DELETE | `/api/accounts/{id}` | owner | Delete an account |
| PUT | `/api/accounts/{id}/secret` | owner | Set a new PIN for someone |
| GET | `/api/engines` | bearer | Engine state, install progress, graphics cards |
| POST | `/api/engines/{id}/install` | owner | Install or reinstall `ollama` / `comfyui` (background) |
| POST | `/api/engines/{id}/start`, `/stop` | owner | Start or stop an engine |
| GET | `/v1/models` | bearer | OpenAI-compatible model list: Ollama models plus `persona:<slug>` |
| POST | `/v1/chat/completions` | bearer | OpenAI-compatible chat; `model` can be `persona:<slug>` |
| GET | `/api/hardware`, `/api/models` | bearer | Hardware tier, scored catalogue, installed models, downloads |
| POST | `/api/models/pull`, `/api/models/remove` | owner | Download or remove an Ollama model |
| GET, POST, PATCH, DELETE | `/api/personas[/{id}]` | bearer | My personas |
| GET, POST, PATCH, DELETE | `/api/chats[/{id}]` | bearer | My chats |
| POST | `/api/chats/{id}/messages` | bearer | Send a message (+ up to 4 `images`, base64); reply streams as Server-Sent Events |
| GET, POST, DELETE | `/api/memory[/{id}]` | bearer | Memories (`source`: `manual` or `auto`) |
| GET, PUT | `/api/memory/settings` | bearer | Learning on or off; which embedding model recall uses |
| DELETE | `/api/memory/auto` | bearer | Forget everything learned from chats |
| GET | `/api/files/{id}` | bearer | An attached image (your own only) |
| GET, POST, DELETE | `/api/keys[/{id}]` | bearer | App keys (owner can make them for service accounts) |
| GET, PUT | `/api/settings/network` | bearer; PUT owner on this PC | Allow other devices on the network |
| GET | `/api/packs` | bearer | Workflow packs, their inputs, model status, fit for this PC, `task`, whether it's the `recommended` pick, and `missing_nodes` (needs a newer ComfyUI) |
| POST | `/api/packs/{id}/install` | owner | Download a pack's models (background) |
| GET | `/api/loras` | bearer | LoRA files available to packs |
| GET, POST | `/api/jobs` | bearer | My jobs (queue position, live progress) / queue one: `{pack, inputs}` |
| GET, DELETE | `/api/jobs/{id}` | bearer | One job / cancel or stop it |
| GET | `/api/queue` | bearer | Everyone's running and waiting jobs in order: yours in full, others' only by name and progress |
| GET | `/api/gallery` | bearer | My pictures and videos, newest first; `?account_id=` for someone else's (owner only) |
| GET, DELETE | `/api/media/{id}` | bearer | One gallery file (yours, a shared one, or for the owner anyone's) / delete it (yours; the owner: anyone's) |
| GET | `/api/gallery/shared` | bearer | The shared gallery, most recently shared first, with who shared each |
| POST | `/api/media/{id}/share` | bearer | `{"shared": true}` to share one of yours with everyone, `false` to stop (the owner can unshare anyone's) |
| GET | `/api/downloads` | bearer | Every download, running and finished: file n of m, bytes, speed, time left |
| POST | `/api/pairing` | bearer, desktop app | A pairing code, QR (SVG), check words and the certificate fingerprint |
| GET | `/api/pairing/{id}` | bearer, desktop app | Has that code been used, and by which phone |
| POST | `/api/pair/hello` | none, encrypted port | Phone step 1: check the code, list accounts |
| POST | `/api/pair/complete` | none, encrypted port | Phone step 2: code + account + PIN → device token |
| GET, DELETE | `/api/devices[/{id}]` | bearer | My paired phones (`?all=true` everyone's, owner) / unpair |
| GET | `/api/connection` | bearer | Where the encrypted port answers now (home address, Tailscale), for phones |
| GET | `/openapi.json` | none | The full spec |

Security notes: PINs and passwords are hashed with Argon2id; tokens are random 256-bit
values stored only as SHA-256 hashes; five wrong attempts lock an account for 30 seconds,
doubling up to 15 minutes. The gateway listens on `127.0.0.1` unless the owner turns on
network access, and even then only app keys work from other machines.

## Clean-room rule

Outpost takes ideas from Locally Uncensored (AGPL-3.0) but **no code**. Do not copy,
port or paste from LU or any other AGPL project into this repository.

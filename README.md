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
- **Downloads**: the button at the top right lists every download (chat models and Create
  files) with the file it's on, speed and time left.
- **LoRAs**: put `.safetensors` files in `<data folder>\models\comfyui\loras` and pick them in
  the form (Characters, or a style LoRA in Z-Image Turbo and Qwen-Image). Start the prompt
  with the LoRA's trigger word.

## Training spike

Before Training Studio (phase 4) is built, `docs/spike-ai-toolkit.md` checks that
ai-toolkit trains Z-Image Turbo and Wan 2.2 on a 16 GB card. Its scripts live in
`scripts/spike/` (install, start, and a captioner that uses Outpost's vision model).

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
| GET | `/api/gallery` | bearer | My pictures and videos, newest first; `?account_id=` for someone else's (owner only) |
| GET, DELETE | `/api/media/{id}` | bearer | One gallery file (yours, a shared one, or for the owner anyone's) / delete it (yours; the owner: anyone's) |
| GET | `/api/gallery/shared` | bearer | The shared gallery, most recently shared first, with who shared each |
| POST | `/api/media/{id}/share` | bearer | `{"shared": true}` to share one of yours with everyone, `false` to stop (the owner can unshare anyone's) |
| GET | `/api/downloads` | bearer | Every download, running and finished: file n of m, bytes, speed, time left |
| GET | `/openapi.json` | none | The full spec |

Security notes: PINs and passwords are hashed with Argon2id; tokens are random 256-bit
values stored only as SHA-256 hashes; five wrong attempts lock an account for 30 seconds,
doubling up to 15 minutes. The gateway listens on `127.0.0.1` unless the owner turns on
network access, and even then only app keys work from other machines.

## Clean-room rule

Outpost takes ideas from Locally Uncensored (AGPL-3.0) but **no code**. Do not copy,
port or paste from LU or any other AGPL project into this repository.

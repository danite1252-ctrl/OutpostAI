# Outpost

Your private AI outpost: a Windows (then Linux) app that turns one PC into a local AI
server for a household or small team. Chat, personas, images, video and training, all
on your own GPU, all reachable over one API so phones and apps like Hermes can plug in.

Everything runs locally. No cloud models, accounts or services. MIT licensed.

> Status: **phase 1, chat.** Accounts with PINs, the local gateway, the API spec,
> the desktop shell, and managed installs of Ollama and ComfyUI. See the build plan for what comes next.

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
| POST | `/api/chats/{id}/messages` | bearer | Send a message; reply streams as Server-Sent Events |
| GET, POST, DELETE | `/api/memory[/{id}]` | bearer | Things to remember |
| GET, POST, DELETE | `/api/keys[/{id}]` | bearer | App keys (owner can make them for service accounts) |
| GET, PUT | `/api/settings/network` | bearer; PUT owner on this PC | Allow other devices on the network |
| GET | `/openapi.json` | none | The full spec |

Security notes: PINs and passwords are hashed with Argon2id; tokens are random 256-bit
values stored only as SHA-256 hashes; five wrong attempts lock an account for 30 seconds,
doubling up to 15 minutes. The gateway listens on `127.0.0.1` unless the owner turns on
network access, and even then only app keys work from other machines.

## Clean-room rule

Outpost takes ideas from Locally Uncensored (AGPL-3.0) but **no code**. Do not copy,
port or paste from LU or any other AGPL project into this repository.

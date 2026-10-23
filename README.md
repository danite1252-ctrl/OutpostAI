# Outpost

Your private AI outpost: a Windows (then Linux) app that turns one PC into a local AI
server for a household or small team. Chat, personas, images, video and training, all
on your own GPU, all reachable over one API so phones and apps like Hermes can plug in.

Everything runs locally. No cloud models, accounts or services. MIT licensed.

> Status: **phase 0, foundations.** Accounts with PINs, the local gateway, the API spec,
> the desktop shell, and managed installs of Ollama and ComfyUI. See the build plan for what comes next.

## Layout

| Path | What it is |
| --- | --- |
| `crates/outpost-core` | Database, accounts, PIN hashing, sessions, audit log |
| `crates/outpost-engines` | Installs, starts and supervises Ollama and ComfyUI |
| `crates/outpost-gateway` | The HTTP API on `127.0.0.1:7420` (library + `outpost-gateway` binary) |
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

## API (phase 0)

| Method | Path | Auth | What |
| --- | --- | --- | --- |
| GET | `/api/health` | none | Up? Version, PC name, whether setup is needed |
| POST | `/api/setup/owner` | none, this PC only | Create the first owner and sign in |
| GET | `/api/accounts/directory` | none | Accounts for the sign-in screen |
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
| GET | `/v1/models` | bearer | OpenAI-compatible model list (from Ollama) |
| GET | `/openapi.json` | none | The full spec |

Security notes: PINs and passwords are hashed with Argon2id; tokens are random 256-bit
values stored only as SHA-256 hashes; five wrong attempts lock an account for 30 seconds,
doubling up to 15 minutes. The gateway listens on `127.0.0.1` only until phone pairing
lands in phase 3.

## Clean-room rule

Outpost takes ideas from Locally Uncensored (AGPL-3.0) but **no code**. Do not copy,
port or paste from LU or any other AGPL project into this repository.

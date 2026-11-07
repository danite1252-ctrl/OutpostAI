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
- **How much of the chat it keeps in mind** (per persona): Large (the most your card allows),
  Medium (32K) or Small (16K). Smaller keeps a big model on the graphics card, so it answers
  faster; the start of very long chats is summarised sooner. Coding personas want Large.
- **Ready-made personas** (Personas › Ready-made, or on the phone): Assistant, Coder,
  Researcher, Devil's Advocate (questions your plans), Writing Coach and Translator, plus fun
  ones: Rizz Coach, Roast Master, Supervillain and Mr Sarcasm. Adding one makes it yours to
  change, and gives it the library tools that suit it (search for the Researcher, say) when
  you have them.
- **Draw from chat**: `/draw a lighthouse at dusk`, `draw Kat at the beach` (with a
  character's name), `4 pictures of Kat at the beach`, or several scenes separated by `;`.
  The picture appears in the chat and the Gallery.

### Bigger chat models on a lent PC

With Outpost on a server whose card is small, a desktop PC can lend its card for chat: on that
PC, **Settings › Server › Lend this PC's graphics card**, keep **Run chats here too** on, and get
a chat model there (it suggests ones that fit). It shows in every model list as **runs on <that
PC>**, and chats with it run on that card, with a longer memory of the conversation. Pause
lending while gaming and those models are unavailable until you carry on. See
[the server guide](linux-server.md#helper-pcs-bigger-pictures-and-smarter-chat-than-the-servers-card-can-run).

### The + menu

Everything that goes with a message is under **+** beside the message box (hover or click):
pictures from this PC or your gallery, documents, files and folders, and tools. What's on for
the chat shows beside it (a folder's name, the tools in use); click one to change it.

### Tools (MCP)

Chats can use tools: web search, reading pages, Google, Home Assistant, GitHub, a hidden web
browser, Wikipedia, Todoist, Notion and more. They're [MCP](https://modelcontextprotocol.io)
servers, so any MCP server works.

- **Where a tool runs**: when you add one, choose **The Outpost server** (it then works from
  every device you sign in on, the phone included, with no PC left on) or **This PC only** (for
  tools that use that PC's own folders or browser, like Filesystem, Git and the web browser).
  **Move to the server** moves a PC tool across, keys, sign-in and chats' choices included.
- **Yours, or everyone's**: tools on the server are personal: SJ adds and signs in to her own
  Google; nobody else can use yours or see its keys. An owner can **Share with everyone** (Home
  Assistant, search), and allow **Restricted accounts too** (the kids). Only owners can add tools
  that run programs on the server.
- **Who can use which actions**: under a shared tool, **Who can use it, and which actions** lists
  everyone else: **All of it**, **Only some** (pick their actions: SJ can turn lights and the AC
  on and off but not unlock doors; the kids can ask what's on but not change the heating) or
  **None**. Press **Test** first to list the tool's actions. Outpost checks this on every call,
  so a model can't use an action someone isn't allowed, and the persona is told what they can't
  do, so it says to ask you instead. Restricted accounts get none of a tool unless you pick for
  them (or turn on Restricted accounts too).
- **Keys and sign-ins on the server** are encrypted with a key kept in Outpost's data folder
  (`secrets.key`), beside the database: keep it in backups, or everyone signs in again.
- **Signing in** happens on the PC you're using (the browser comes back to it), and the sign-in
  is then kept on the server, renewed automatically. **Google**: the household's Google client
  (made once, below) is saved by the owner who adds it first; everyone else just signs in.
- **On the phone**: + › Tools turns the chat's server tools on and off. **You › Tools** lists your
  tools and the household's, and **Add a tool** opens the library. Signing in opens the phone's
  browser and comes back to Outpost; the sign-in is kept on the server like one made on a PC.
  Members can add any tool at an address; tools that run a program on the server are added by
  an owner (and shared), and ones that use a computer's own files from the desktop app.

- **+ › Tools › Tool library** lists ones that are known to work, grouped (Search & the web,
  Coding, Knowledge, Home & tasks, Helpers), each saying what it costs (most are free; some need
  a key, with a link to get one). **Add and test** starts it and lists its tools. Good first
  ones: DuckDuckGo Search, Fetch, Wikipedia, Context7 and GitHub.
- Tools that run with `npx` or `uvx` need Node.js or Python: if your PC hasn't got them, the
  library offers to download them into Outpost's own folder (nothing is installed system-wide).
- **Add your own**: any MCP server's command (or address and headers) from its instructions.
- **Signing in (OAuth)**: services that sign you in on their own page (Notion, Linear, Google
  and others) show **Sign in**: your browser opens at their page, and once you've signed in it
  comes back to Outpost on this PC (a one-off address like `http://127.0.0.1:52344/callback`).
  Outpost registers itself with services that allow it; for ones that don't (Google), make an
  OAuth client as the library's steps say, with `http://127.0.0.1:33418/callback` as its
  redirect address, and enter its client ID. Tokens stay on this PC and
  are renewed automatically; **Sign out** forgets them.
- **Google** (built in): Drive, Docs, Sheets, Gmail and Calendar, with a personal or work
  Google account. Make a free login client once in Google Cloud (the library lists the steps:
  a project, the five APIs switched on, the consent screen published, and a "Desktop app"
  client), then **Sign in with Google**. Google says the app isn't verified, because it's your
  own: **Advanced › Go to Outpost**. Under **On this PC** choose which parts it may use and
  whether it may change things; mail is only ever written as drafts for you to send.
- **Which chats use which**: a persona's tools are set in **Personas** (new chats with it start
  with those); a chat can be given its own set from **+ › Tools**. **On this PC › Test** shows a
  tool server's tools; switch off any you don't want used.
- The chat's model needs the **tools** badge. Under each reply, **Used tools** lists what it
  did. Tools run only while the desktop app is open on that PC (even in the tray): keys stay on
  the PC, and the server only sees each tool's name and description.
- **From the phone**: your phone's chats use the same tools, run on your PC, while the
  Outpost app is open there. Each chat uses its persona's tools (or its own set, chosen on the
  PC). Only your own PC's tools, for your own chats.
- **Hands-free asks first**: talking hands-free (on the phone or the PC), every tool call is
  read back before it runs ("Would you like me to check Google Calendar?", "Would you like me
  to turn off: kitchen lamp, with Home Assistant?"). Say yes (or "go ahead") to run it;
  anything else, or no answer within 20 seconds, cancels it, and the model asks what you
  meant.
- Tools act straight away (unlike a chat's folders, they don't ask first), so give keys only
  the access you're happy for the model to use (e.g. GitHub's read-only switch).

### Files in chats

A chat can work with folders: the model lists, reads and searches the files, and can write
or change them. It needs a model with the **tools** badge (Qwen 3, Llama 3.1 and similar).

- **Files** (chat header) shows the chat's folders. Add up to 8:
  - **A folder on this PC** (desktop app): Windows' folder picker. The files stay on your PC;
    while a reply runs, the app reads and writes them for it, so that PC's Outpost app must
    be open. The chat sees only the folder's name, never where it is.
  - **A server folder**: upload a folder (or start an empty one) and it lives on your Outpost,
    usable from any device, including the phone. Download it as a .zip any time.
- **Ask before changing files** (on by default, set per chat): each change the model makes
  waits under its reply with **Show change**, **Apply** and **Skip**. Turn it off and changes
  are saved straight away, each with **Undo**. If the file changed in between, Outpost asks
  before overwriting it.
- Under each reply, **Looked at files** lists what it read or searched.
- On the phone you can add and remove server folders, switch asking on or off, and apply,
  skip or undo server-folder changes. A PC folder's changes are applied from the desktop app
  on that PC.
- The model only reaches inside the chat's folders; paths that leave them (`..`, links out)
  are refused. Folders like `.git` and `node_modules` are skipped when listing and searching.

#### Building and running commands (coding)

In a chat with a folder, the model can also run commands there, such as `gradlew build`,
`mvn package` or `npm test`, then read the errors, fix the code and build again until it
works (up to 30 rounds in one reply).

- **A folder on a PC**: a command runs on that PC, in that folder, as you (Java, Gradle and
  the rest come from that PC). Carry on the chat from your phone or another PC and it still
  runs there, as long as that PC is on with the Outpost app open.
- **A server folder**: a command runs on the Outpost server (Linux), so a chat started on the
  phone can build without a PC. Outpost downloads Java (Temurin 21, or 25 when the build files
  ask for it) and Gradle itself the first time, so a chat can start a new plugin in an empty
  folder; a project's own wrapper (`gradlew`) is used when it has one. When a build makes a `.jar` (in `build/libs` or `target`), it's
  shown under the reply with **Download** (on the phone it's saved in Download/Outpost).
  - With **bubblewrap** installed on the server (`sudo apt install bubblewrap`), commands run
    sandboxed: they see only that folder, Java and a build cache, nothing else on the server.
    Then members can run commands too; without it, only owners can.
- **Asking first**: a command shows **Allow**, **Always allow … here** (that program, in that
  folder, from then on) and **Skip**. Commands chained together (`&&`, `|`, `>`) always ask,
  and shells like `cmd` and `powershell` can't be always allowed. A few commands never run
  (formatting drives, deleting folders of files, shutting down).
- A command is stopped after 10 minutes (the model can ask for up to 30), with everything it
  started.
- **Things that keep running** (a test Paper server, a dev server): the model starts them
  in the background (asking first, like a command), reads their log, types console commands
  into them (`plugins`, `stop`) and stops them when it's done. **Files** shows what's still
  running in each folder, with **Stop**. Anything left running is stopped after 3 hours.
- **Cleaning up**: server folders stay on the server until you delete them (a folder can be
  in several chats). Deleting a chat offers to delete its server folders too when no other
  chat uses them. **Files** lists all your server folders with their sizes and a **Delete**
  (anything running in one stops first). Builds keep the libraries they download so the next
  build is quick; owners see their size and **Clear build caches** under the server's settings
  (Java and Gradle themselves are kept).
- Long output comes back to the model as the error lines and the end, so it fits.
- **Stop** ends the reply at its next step and stops a command it's running (with everything
  it started). What it wrote so far is kept.
- A reply that changed several files has **Apply all** / **Skip all** (asking first) or
  **Undo all** (newest first) under its changes. Files changed since are left for you to do
  one by one.

#### Following a reply on another device

Open a chat on the phone (or another PC) while its reply is being written somewhere else and
you see it as it's written, from the start, including any request to allow a command, which
you can answer there. Stop works from there too.

## Routines

A routine is a persona doing something on its own, on a schedule: a morning brief, checking
whether a new Paper build is out, a weekly summary. Owners and members can make them (on the
PC: **Routines** in the sidebar; on the phone: **You › Routines**).

- **Start from a ready-made one** (New routine): a morning brief, an update checker, news on a
  topic, the week ahead, a reminder, or captions for new creations. Change anything in
  <angle brackets>. Some need tools on their persona (web search, Google).
- **Or ask in any chat**: "remind me at 5 to call Mum", "every weekday at 7 give me the
  weather", "check that page every 6 hours and tell me if it changes". The persona sets it up
  and asks you first (**Make it** / **No thanks**); it runs as that chat's persona, in your
  time zone. Ask "what routines do I have?" to hear them. (Needs a model with tools; owners and
  members.)
- Give it a **name**, a **persona** (its instructions, model and tools do the work) and what it
  should do, in plain words.
- Pick when: **every day**, **weekdays**, **chosen days** at a time, **every few hours**, or
  **once**. Times are your device's time zone (daylight saving included), wherever the server is.
- Or start it **when something happens**: a picture, video or 3D model of yours finishing, or a
  character's training finishing. What finished (its name and how it went) is passed to it. A
  picture the routine makes itself doesn't start it again.
- Or **from a link**: give the routine a secret link (on the PC, **Make its link**, or **Also
  start it from a secret link…** for any routine; on the phone, **Make its link**) and call it
  from n8n, Home Assistant or a script, with POST or GET. Anything sent with it (text, or JSON
  like `{"who": "Sam"}`) is passed to the routine as information. The link is shown once:
  copy it then. **Make a new link** replaces it (the old one stops working); **Remove link**
  takes it away. Outpost's certificate is its own, so tell the caller not to check it (n8n:
  "Ignore SSL issues"; Home Assistant: `verify_ssl: false`).
- Each run is a reply in the routine's own chat, so you can read every result (**Open chat**)
  or carry on talking about one. The list shows when it runs next and how the last run went.
- **Run now** runs it straight away (its schedule carries on as before). **Pause** stops it
  until you resume it. A routine never runs twice at once, and a run is stopped after 30 minutes.
- The list is in sections (on a schedule; reminders and one-offs; from a link or an event),
  each folding away with its arrow. A one-off routine that ran fine clears itself away an hour
  after it told you (its chat stays); one that failed stays, so you can see why.
- Routines use the persona's tools on the server, and its tools on your PC when that PC has
  Outpost open.
- **While it runs on its own**, nobody is there to say OK, so each routine says what it may do:
  use its chat's tools (on by default); make **file changes** in its chat's folders straight
  away (off by default: they wait for you to apply them); and run **commands**, but only ones
  you've allowed **Always** in that folder (to allow one, run it once in the routine's chat and
  choose Always). A command that isn't allowed doesn't run, and the routine says which one
  needs allowing.
- If the routine's model runs on a **lent PC** that's off or paused (or the card is training),
  it waits and tries again every 5 minutes, for up to an hour (the list shows it's waiting),
  then tells you it couldn't run. A scheduled time that comes while it's still waiting is
  skipped.
- **Quiet hours** (at the top of Routines): routines still run, but news from a run that
  finishes in them is told when they end (a 6:30 morning brief with quiet hours until 7:00
  notifies you at 7:00).
- **Tell me**: each routine sends a notification to your phone and PC when it runs (**Every
  time**), **Only when there's news** (the persona decides; when there's nothing new it says so
  quietly in the chat, and a quick separate check of each result decides; failures are always told), **Only if it fails**, or **Never**.
  Tapping the notification on the phone opens the routine's chat. The phone looks just after
  each routine is due (and every few minutes while one is running), as well as every 15
  minutes or so. If phone notifications come late or not at all, let Outpost run in the
  background: on Samsung phones, Settings › Apps › Outpost › Battery › **Unrestricted** (and
  keep it out of "Sleeping apps"). On the PC, notifications need
  **Tell me when long jobs and routines finish** on in Settings.

## Create, Gallery and the queue

- **Create** has a tab per task: **Picture**, **Character**, **Edit**, **Video** and
  **Animate**. Outpost picks workflows that suit your graphics card and downloads their
  model files the first time (the **Downloads** button shows progress; **Cancel** there stops one you started by mistake,
  and getting it again later carries on from where it stopped).
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

**You › Your Outpost** on the phone covers the day-to-day running of it, with no PC needed:

- **Personas**: make and change them (instructions, model, tools, documents, voice, sharing),
  send someone a copy, or make your own copy of one shared with you.
- **Tools**: add, sign in to, test and share tools (see Tools above).
- **Documents and memory**: your documents (add one by sharing a file to Outpost from any
  app) and what your personas remember. A chat's documents are in its + menu.
- **Models**: what's downloaded (and on a lent PC); owners download, cancel and remove.
- **People and devices**: everyone sees and unpairs their own phones, and can pair another.
  Owners invite people (a QR code, or a link sent with any app), send a new-PIN link, and
  remove accounts.
- **Outpost server** (owners): its version, installing an update, automatic updates, a
  restart, and crash reports.

Risky actions (removing models, inviting, restarting) ask for your PIN again on the phone.

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

Updating or restarting a server asks first when something is running that it would stop: a
reply being written, a Create job, a build or a test server. Automatic updates wait until
nothing is running.

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

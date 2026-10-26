# Phase 5c live test: installer, updates, crash reports, docs

The gate: **Outpost installs from the installer on a clean Windows user, updates itself
from a GitHub release after you press Update, and a crash leaves a report you can read,
with nothing sent anywhere.**

## 1. Crash reports and logs (development build)

Start Outpost (`npm run dev`) and sign in as the owner.

1. **Settings › Diagnostics**: "No crash reports", **Show recent log** shows today's log
   lines, **Copy log folder path** copies `…\AppData\Local\Outpost\logs`.
2. That folder has `outpost.log`, growing as you use Outpost.
3. Make a test crash report: in PowerShell,

   ```powershell
   $d = "$env:LOCALAPPDATA\Outpost\crash-reports"
   "Outpost hit an error`n`nWhat: test report`n" | Out-File "$d\crash-$([DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()).txt" -Encoding utf8
   ```

   Restart Outpost. **Pass:** a note at the top says Outpost had a problem last time;
   **View it** opens Settings with the report listed; **View** shows it, **Copy** copies it,
   **Delete** removes it, and the note doesn't come back.

## 2. Updates (development build)

**Settings › Updates**: "This is Outpost 0.1.0". **Check now** says *No release has been
published yet* (until the first release exists). Untick **Check for updates**, restart:
it stays off. Tick it again.

## 3. The installer

Runner job `installer` builds `target\overnight\Outpost-test-setup.exe`.

1. Close the development Outpost. Run the installer. **Pass:** SmartScreen may warn
   (More info › Run anyway); it installs without asking for administrator rights and adds
   **Outpost** to the Start menu.
2. Open it from the Start menu. **Pass:** it opens on *Who's using Outpost?* with your
   accounts, engines and models (same data folder), and works as in development.
3. **API** page: the Claude Desktop config shows `…\AppData\Local\Outpost\outpost-gateway.exe`
   (the app installs next to its data).
4. Uninstall it (Windows Settings › Apps), then look in `%LOCALAPPDATA%\Outpost`:
   **Pass:** `Outpost.exe` is gone, `outpost.db`, `files` and `models` are still there.
   Install it again.
5. Close Outpost with the window's ✕ and reopen it: no "problem last time" note. Then end
   it in Task Manager and reopen: the note says it **didn't close normally** last time.

## 4. First release and a real update (the gate)

Needs the update key, the phone key and the public repo (see `docs/releasing.md`).

1. `release.ps1 -Version 0.1.1 -Notes "First release"` → publish → install
   `Outpost_0.1.1_x64-setup.exe` (over the test install).
2. `release.ps1 -Version 0.1.2 -Notes "Testing updates"` → publish.
3. In the installed 0.1.1: **Settings › Updates › Check now**. **Pass:** "Outpost 0.1.2 is
   ready" with the notes, and the bar at the top. **Update and restart** downloads,
   installs and reopens as 0.1.2, still signed in to the same data.
4. Phone: install `Outpost-0.1.1.apk` (uninstall the test app first, once), pair. After
   0.1.2 is published, **You** shows "Outpost 0.1.2 for this phone is out"; **Download the
   new app** installs over it and you stay paired.

## 5. Docs

`docs/user-guide.md` reads well for someone new: install, first start, each part of the
app, privacy, data and backups, troubleshooting. `docs/releasing.md` is enough to make a
release without help.

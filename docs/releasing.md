# Making an Outpost release

Releases live on [github.com/danite1252-ctrl/OutpostAI/releases](https://github.com/danite1252-ctrl/OutpostAI/releases).
Each one has:

| File | What |
| --- | --- |
| `Outpost_<version>_x64-setup.exe` | The Windows installer (per-user, no admin) |
| `Outpost_<version>_x64-setup.exe.sig` | Its update signature |
| `Outpost-<version>.apk` | The phone app, signed with the release key |
| `outpost-gateway-linux-x86_64` | The Linux server program |
| `outpost-gateway-linux-x86_64.sig` | Its update signature (servers check it before installing) |
| `install.sh` | Installs or updates Outpost on a Linux server |
| `latest.json` | What installed copies read to find the update (and the phone app) |

Installed copies of Outpost read
`https://github.com/danite1252-ctrl/OutpostAI/releases/latest/download/latest.json` once a day,
so **the newest non-draft, non-prerelease release is the one everybody is offered**.

## Once: keys

Both keys live in `%USERPROFILE%\.outpost`, never in the repo. **Back them up with their
passwords**: if the update key is lost, installed copies can't accept updates from a new
one (people would have to reinstall); if the phone key is lost, phones have to uninstall the
app (and pair again) to take a new one.

1. **Update key** (desktop):

   ```powershell
   cd C:\dev\Outpost\apps\desktop
   npx tauri signer generate -w $HOME\.outpost\updater.key
   ```

   Put the contents of `$HOME\.outpost\updater.key.pub` in
   `apps/desktop/src-tauri/tauri.conf.json` → `plugins.updater.pubkey`, and commit that.

2. **Phone release key**:

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts\release\make-keys.ps1
   ```

   Phones that had a test build (signed with the debug key) need to uninstall it once
   before installing a release build.

3. Optional: the **GitHub CLI** to publish from the script: `winget install GitHub.cli`,
   then `gh auth login`.

## Each release

1. Commit and push everything (the script refuses uncommitted changes).
2. **Build** (about 10–15 minutes; asks for both key passwords):

   ```powershell
   cd C:\dev\Outpost
   powershell -ExecutionPolicy Bypass -File scripts\release\release.ps1 -Version 0.2.0 -Notes "Voice, hands-free and updates."
   ```

   It sets the version everywhere (`scripts/release/bump.mjs`), builds the gateway (it ships
   beside the app for the MCP bridge), the installer (signed for updates), the Linux server
   (built on the server by its runner, then signed with the same update key), the phone app
   (signed with the release key) and `latest.json`, into `target\release-out\v0.2.0`.
   `-SkipPhone` leaves the phone app out; `-SkipLinux` the Linux server (needed if the
   server's runner isn't running).
3. **Commit and push the version change**: `git add -A; git commit -m "Outpost 0.2.0"; git push`.
4. **Publish** (needs `gh`): the same command with `-PublishOnly` on the end. It makes the
   release `v0.2.0` on GitHub, tagged on the commit you just pushed, with every file attached.
   Without `gh`: make a release named `v0.2.0` on GitHub by hand and attach every file in
   `target\release-out\v0.2.0`.

Servers (started with `--server`) also see it within a day, and an owner installs it from
any paired device with **Update the server** (the server checks the signature, swaps itself
and restarts).

Within a day every installed Outpost shows "Outpost 0.2.0 is ready" (or straight away with
**Settings › Updates › Check now**), and paired phones show the new app under **You**.

## Android developer registration

Google requires apps installed outside the Play Store to come from a registered developer
(from 30 Sep 2026 in Brazil, Indonesia, Singapore and Thailand; everywhere in 2027). Outpost
is registered in the Android Developer Console (free **limited distribution** account: up to
20 devices, which suits a household; switching to a full account, US$25 with ID, lifts that) with:

| | |
| --- | --- |
| Package name | `app.outpost.mobile` |
| Release key certificate (SHA-256) | `B4:91:A4:EE:B5:41:8C:99:49:AF:89:B6:87:66:53:CE:08:F0:B2:BF:5D:5D:FE:DE:9A:72:16:9C:B2:6E:D3:4C` |

The fingerprint is public (it identifies the key without revealing it). Every release must be
signed with this same key (`%USERPROFILE%\.outpost\outpost-release.keystore`).

## Notes

- The installer isn't Authenticode-signed (no paid certificate yet), so Windows SmartScreen
  shows "unknown publisher" on first install. Updates are still checked against the update
  key before they install.
- Versions follow `MAJOR.MINOR.PATCH`. A version with a suffix (`1.0.0-beta.1`) counts as
  older than the same version without one.
- `target\overnight\Outpost-test-setup.exe` (from the test runner's `installer` job) is a
  local test build: not signed for updates, and never to be published.

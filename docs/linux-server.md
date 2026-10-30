# Outpost on a Linux server

Outpost can run on a home server with no screen (for example an Ubuntu VM with the graphics
card passed through). The server holds everything: accounts, chats, pictures, models and the
engines. Desktop PCs and phones use it as **paired devices**.

## Installing

On the server, as the user Outpost should run as:

```bash
curl -fsSL https://github.com/danite1252-ctrl/OutpostAI/releases/latest/download/install.sh | bash
sudo loginctl enable-linger $USER   # once: keep it running when you're logged out, and at boot
```

It installs the newest release (checking its SHA-256), sets up the service and shows the setup
code. Running it again updates Outpost the same way.

## Running it

Outpost runs as a systemd user service (`scripts/linux/outpost.service`) with `--server`:

- **Network access is on** until the owner turns it off. Other devices are the only way in,
  so they can reach the encrypted port (7421) from the start.
- Data lives in `~/.local/share/Outpost`.
- With `loginctl enable-linger <user>` it starts at boot without anyone logging in.

```bash
systemctl --user status outpost       # is it running?
journalctl --user -u outpost -f        # its log
```

### Firewall

If the server uses `ufw`, allow Outpost's ports from your home network only:

```bash
sudo ufw allow from 192.168.1.0/24 to any port 7421 proto tcp   # paired devices (encrypted)
sudo ufw allow from 192.168.1.0/24 to any port 7420 proto tcp   # app keys (plain HTTP; optional)
```

Change `192.168.1.0/24` to your network. Port 7420 is only needed for apps that use app keys
from other machines; paired devices only use 7421.

## First-time setup (no owner yet)

1. On the server, get the setup code:

   ```bash
   ~/.local/lib/outpost/outpost-gateway --setup-code
   ```

   It shows an 8-digit code, how long it lasts, and three **check words**. The code works
   once, lasts 30 minutes (a new one replaces it by itself), and five wrong tries replace it.
   It's also in the log and in `~/.local/share/Outpost/setup-code.txt` (readable by you only).
2. In the Outpost desktop app on a PC, choose **Connect to an Outpost server** (on the
   first-run screen, under the account picker, or in **Settings › Server**). Type the server's
   address, for example `192.168.1.98`.
3. Check the app shows the **same three words** as the server. Different words mean something
   else is answering at that address: stop there.
4. Enter the setup code, your name and a PIN. That makes the owner account, and this PC
   becomes its first paired device. The app restarts using the server.

## More devices

On any paired device (or the server's own desktop app), open **Connections › Pair a device**.
That shows a six-digit code, a QR code and the check words (a paired device asks for its PIN
first):

- **Phones** scan the QR code in the Outpost app. A phone without the app can scan it with its
  camera: that opens a page (on GitHub Pages) to download the app, then hands the code to it.
  The pairing details are after the `#` in that link, which never leaves the phone.
- **Another PC**: in its desktop app choose **Connect to an Outpost server**, type the server's
  address, check the words, then type the code, pick the account and enter its PIN.

A paired device stays signed in to its account until it's unpaired (Connections, by its owner or
the server's owner). It asks for the PIN again only before risky changes: deleting models or
accounts, installing engines, making app keys, pairing devices, and changing network access.

## Adding people

In **Settings › Accounts**, an owner adds someone with **Make an invite** (the default): Outpost
shows a code, a QR code and the check words, valid for 24 hours. The person scans it with the
phone app, or types it in the desktop app's **Connect to an Outpost server**, checks the words,
and picks their own PIN. That makes their account and pairs that device. Nobody needs to log in
to the server.

**Send PIN reset** does the same for someone who forgot their PIN: they pick a new one on their
own device, and their other devices are signed out.

### A shared PC

A desktop app using a server can hold several people (**Add someone to this PC**, from the lock
button or Settings › Server). With more than one, it opens on **Who's using Outpost?** and asks
each person's PIN, and locks back to it after 15 idle minutes. With one, it stays signed in.

### Locked out

If the only owner forgets their PIN, run this on the server:

```bash
~/.local/lib/outpost/outpost-gateway --reset-pin Dan
```

It prints a reset code and the check words (valid 24 hours); use it in the desktop or phone app
like an invite and pick a new PIN.

## Helper PCs: bigger pictures than the server's card can make

A desktop PC using the server can lend its graphics card for Create jobs the server's own card
is too small for. On that PC (signed in as an owner), open **Settings › Server › Lend this PC's
graphics card**:

- Tick **Lend this PC for Create jobs**. Install ComfyUI there if it asks, then **Download on
  this PC** the workflows listed as too big for the server.
- Create on every device then lists those workflows as **Runs on <that PC>**. Their jobs wait in
  the server's queue and go to the PC when it's free; the results land in the gallery as usual.
- Only ComfyUI runs on the helper, and only while there's a job (its models are unloaded after
  five idle minutes). Chat stays on the server.
- **Pause** (in Settings, or **Pause lending this PC** in the tray icon's menu) while gaming:
  ComfyUI stops, and the queue shows "Waiting for <that PC>" until you carry on.
- Several PCs can lend at once; each job goes to one that has the workflow and fits it. If a
  helper is switched off mid-job, the job fails after two minutes with a note saying so.

## Updates

The server checks for a new release a minute after it starts and once a day. When there is one,
owners see it in the desktop app (the banner, and Settings › Updates): **Update the server**
downloads it, checks its signature against Outpost's release key, swaps it in and restarts.
The desktop app checks for its own update at start and once a day too. **Check now** looks at
once. **Settings › Server › Restart the server** restarts Outpost (e.g. after a network change).

## Away from home: Tailscale

Install Tailscale on the server and sign it in to your tailnet:

```bash
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Then restart Outpost (Settings › Server › Restart the server). A server answers on its
Tailscale address automatically (Connections shows it; turn off "Allow my Tailscale devices"
there if you don't want it), and phones paired from then on, or already paired, learn the
address and use it when they're away from home.

## Going back

**Settings › Server › Use Outpost on this PC instead** makes the desktop app run Outpost on that
PC again, with the data it had there before (nothing is moved or deleted either way). The server
stays saved, still paired: **Go back to Outpost** (in Settings › Server, or under the account
picker) switches back with no code. **Forget it** clears it, so connecting again means pairing.

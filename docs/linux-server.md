# Outpost on a Linux server

Outpost can run on a home server with no screen (for example an Ubuntu VM with the graphics
card passed through). The server holds everything: accounts, chats, pictures, models and the
engines. Desktop PCs and phones use it as **paired devices**.

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

- **Phones** scan the QR code in the Outpost app.
- **Another PC**: in its desktop app choose **Connect to an Outpost server**, type the server's
  address, check the words, then type the code, pick the account and enter its PIN.

A paired device stays signed in to its account until it's unpaired (Connections, by its owner or
the server's owner). It asks for the PIN again only before risky changes: deleting models or
accounts, installing engines, making app keys, pairing devices, and changing network access.

## Going back

**Settings › Server › Use Outpost on this PC instead** makes the desktop app run Outpost on that
PC again, with the data it had there before (nothing is moved or deleted either way).

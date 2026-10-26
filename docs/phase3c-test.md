# Phase 3c live test: away from home (Tailscale)

About 10 minutes. Tailscale must be running and signed in on the PC and the phone.

## 1. Turn it on

1. `npm run dev`, then **Connections** › Network › **Allow my Tailscale devices**.
   The line under it shows this PC on Tailscale (its MagicDNS name and 100.x address).
2. Close and reopen Outpost. Windows may ask about the firewall again: allow it.

**Pass:** the switch shows a green dot, and "Phones try, in order:" lists your home
address, then the Tailscale name, then the 100.x address.

## 2. Let the phone learn the new addresses

Open the Outpost app on the phone **at home** (on Wi-Fi) once. It picks up the Tailscale
addresses by itself; no need to pair again.

## 3. Away from home

1. Turn the phone's Wi-Fi off (mobile data on, Tailscale app connected).
2. Open Outpost on the phone and use Chat, then the Gallery.

**Pass:** it works on mobile data. The first request may take a few seconds (it tries the
home address first, then switches to Tailscale and remembers it). The red "Lost
connection" bar shows only if Tailscale is off on the phone.

## 4. Tailscale only

Turn **off** "Allow other devices on my network" (leave Tailscale on), restart Outpost.
Hermes on another machine now needs its Tailscale address (`http://100.x.y.z:7420/v1`);
the phone keeps working over Tailscale, on Wi-Fi or not.

## 5. Gallery pictures in chat

- **Desktop:** Chat › **From gallery** › pick a picture › ask a vision model about it.
- **Phone:** Chat › **＋** › **From gallery** › pick one › send.

## What to send Claude

Anything that failed, and roughly how long the first request took away from home.

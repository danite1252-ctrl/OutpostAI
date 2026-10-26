# Phase 3b live test: the Android app

About 20 minutes once the app is built. You need the Outpost PC running with network
access on (as in 3a), and the phone on the same Wi-Fi.

## 0. Get the app on the phone

The test runner builds it (`target\overnight\outpost-test.apk`) and, if the phone is
plugged in with USB debugging on, installs it. Otherwise copy the APK to the phone and open
it (allow "Install unknown apps" for your file manager when asked).

It's a test build with everything inside it (no development server needed), signed with
a test key. If something crashes, screenshot it for Claude.

## 1. Pair by QR

1. On the PC: Connections › **Pair a phone**.
2. On the phone: open **Outpost** › **Scan the code on the PC**, allow the camera, point it
   at the QR.
3. Pick your account, enter your PIN, keep or change the phone's name, **Pair**.

**Pass:** the PC dialog says "Paired Samsung SM-S92… for Dan"; the phone opens on Chat.

## 2. Pair by the list (do this on a second account, or unpair first)

Unpair from the phone (You › Unpair) and pair again: this time tap your PC under **On this
network**, check the three words match the PC dialog, type the code.

**Pass:** the words match and pairing works. If the list stays empty, use **Type the
address** with the IP shown in the dialog and tell Claude.

## 3. Unlock

Close the app fully and reopen it: it asks for your fingerprint or face (whatever the
phone uses), then opens. No PIN.

## 4. Chat

Open a chat, send a message: the reply streams in word by word. Attach a photo to a vision
model and ask about it.

## 5. Create, queue, gallery

1. Create › Text to image › a prompt › **Generate**. It jumps to **Queue**: your job with
   steps, percentage and time left.
2. When done, **Gallery** › Mine shows it. Open it: **Edit this picture** fills Create's
   Edit tab.
3. Start a job on the PC as someone else: the phone's Queue shows "Sam is creating" with
   their progress only.

## 6. Protection

1. Turn the phone's Wi-Fi off: you get "Can't reach your Outpost", not a crash.
2. On the PC, Connections › Phones › **Unpair** the S24: the phone says "Signed out" on
   its next request.

## What to send Claude

Screenshots of anything red or odd, and how the streaming and image loading felt.

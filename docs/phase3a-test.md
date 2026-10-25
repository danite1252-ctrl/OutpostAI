# Phase 3a live test: secure pairing

About 10 minutes. The phone app comes in 3b, so a PowerShell script plays the phone.

## 0. Start with network access on

```powershell
cd C:\dev\Outpost
npm run dev
```

Owner: **Connections** (side bar) > Network > **Allow other devices on my network** must be
on. If you just turned it on, close and reopen Outpost. Windows may ask about the firewall:
allow **Private networks** (it covers ports 7420 and 7421).

**Pass:** Connections > Phones shows "Encrypted port 7421 on 192.168.x.x · check words …".

## 1. Pair the pretend phone

1. Press **Pair a phone**. You get a QR code, a 6-digit code, three check words and a
   10-minute countdown.
2. In a second PowerShell window:
   ```powershell
   cd C:\dev\Outpost
   powershell -ExecutionPolicy Bypass -File scripts\phone\pair-test.ps1
   ```
3. It prints three words: they must match the dialog. Type `y`, the code, pick your
   account, type your PIN.

**Pass:** the script says "Paired", "Encrypted port: signed in as …" and "Plain HTTP refused
the phone token". The dialog switches to "Paired PowerShell test for Dan" by itself.

## 2. Codes are one-shot

Run the script again with the same code: it should say the code isn't right or has expired.
Close the dialog, press **Pair a phone** again: a new code.

## 3. Unpair

The phone list shows "PowerShell test", last used just now. Press **Unpair**. Run step 4
of the script by hand if you like; the token is refused straight away.

## 4. Hermes still works

Send Hermes a message as before (plain port 7420, app key): unchanged.

## What to send Claude

- Anything that failed (the script's red text, or a screenshot).
- Whether Windows asked about the firewall, and what you chose.

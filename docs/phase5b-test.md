# Phase 5b live test: voice

The gate: **a full hands-free conversation with a persona on the phone**: you speak,
it answers out loud in the persona's voice, and you carry on without touching the screen.

How the pieces fit:

- **Listening.** On the desktop, the PC's Whisper turns your voice into text. On the
  phone, Android's own speech recognizer does it (on the phone itself where Android can),
  so only text goes to the PC. A phone with no recognizer records a clip and the PC's
  Whisper listens instead.
- **Speaking.** The PC's Kokoro speaks every reply, desktop and phone, in the persona's voice.

## 1. Install voice on the PC

Start Outpost (`npm run dev`). **Settings › Engines › Voice › Install** (about 1 GB:
Python, Whisper "small", Kokoro and its voices).

**Pass:** it reaches *Running*. Restart Outpost: it starts again by itself.

## 2. Give a persona a voice

**Personas** › pick one › **Voice**: choose a voice (e.g. *Heart, American English, woman*)
and press **Hear it**. Tick **Read replies aloud**. Save.

**Pass:** you hear a short line in that voice. A different voice sounds different.

## 3. Desktop: talk, listen, hands-free

In a chat with that persona:

1. Press the **microphone**, say a sentence, press it again. **Pass:** your words appear in
   the box (not sent yet). Send it.
2. **Pass:** the reply is read aloud by itself (because of *Read replies aloud*). The
   **play** button on any earlier reply reads that one; pressing it again stops it.
3. Press **Hands-free**. Say something, then pause. **Pass:** after about a second it sends,
   the persona answers out loud, then it listens again. **Interrupt** cuts the reply short;
   **Pause** stops listening; **End** (or Esc) closes it. The turns are in the chat afterwards.

## 4. Phone: install the new app

The phone needs this build because it now uses the microphone:
`target\overnight\outpost-test.apk` (copy it to the phone and open it, or plug the phone
in with USB debugging on and the runner installs it).

The first time you use the microphone Android asks for permission: allow it.

## 5. Phone: talk to chat and read aloud

Open a chat with the persona from step 2.

1. Press **Mic**, speak. **Pass:** the words appear in the box *while you talk*, and it
   stops by itself when you pause (or press **■**). Edit if you like, then **Send**.
2. **Pass:** the reply is read aloud through the phone's speaker in the persona's voice.
   Long replies start quickly (the first sentence plays while the rest is made).
3. **▶ Listen** under any reply reads it; **■ Stop** stops it.

## 6. Phone: hands-free (the gate)

Press **Talk** at the top of the chat. Hands-free works like a call: the microphone stays
open with the phone's echo cancelling, Outpost notices when you start and stop talking
(no beeps), your PC's Whisper works out the words, and the reply is read aloud.

**Pass, all of:**

- It says *Go ahead*; the circle grows as you speak. No start or stop sounds.
- About a second after you stop it shows *thinking*, then reads the answer out loud, then
  it's ready again by itself.
- Hold a conversation of at least **four turns** without touching the screen.
- **Talk over it:** start speaking while it's answering. The reply stops within about a
  quarter of a second and what you said is answered (the first words aren't lost).
- The persona doesn't interrupt itself: its own voice from the speaker doesn't cut the
  reply short. (If it does on your phone, turn **Talk over replies** off and tell me.)
- **Pause** / **Resume** work; **End** or the back button closes it. The conversation is
  in the chat afterwards.
- `/draw` works by voice too: say "draw Kat at the beach".

The chat's **Mic** button still uses the phone's own recognizer, now without its beeps.

## 7. Things that should fail gracefully

- **Voice not installed / stopped** on the PC: the reply still arrives as text; reading
  aloud says voice isn't set up.
- **Microphone refused**: hands-free pauses and says how to allow it.
- **Silence, a cough, a door**: hands-free keeps listening (nothing is sent).

## 8. Other apps

```powershell
$key = "op_your_app_key"
$h = @{ Authorization = "Bearer $key" }
Invoke-WebRequest -Method Post -Uri http://127.0.0.1:7420/v1/audio/speech -Headers $h -ContentType "application/json" `
  -Body '{"input":"Hello from Outpost.","voice":"nova"}' -OutFile hello.wav
curl.exe -s -H "Authorization: Bearer $key" -F "file=@hello.wav" http://127.0.0.1:7420/v1/audio/transcriptions
```

**Pass:** `hello.wav` plays "Hello from Outpost."; the second command prints
`{"text":"Hello from Outpost."}` (or very close).

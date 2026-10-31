# What's next

Done most recently: the Linux server (phase 6), helper PCs for Create, tidier Settings, the
one-command release (`scripts/release/ship.ps1`), picture tools (upscale and background removal,
from the gallery's picture viewer), the Install ai-toolkit button in Train, and chat with your
documents (PDF, Word, text: attached to a chat or a persona, with sources under each reply),
share to Outpost from any Android app, pairing QR codes that open a download page on a
phone without the app, gallery search by what's in the pictures, automatic server updates, and
a choice of listening model for voice (up to Whisper large-v3-turbo), and 3D models from a
picture (Hunyuan3D 2.1 for a quick shape, TRELLIS.2 for a textured model on a 16 GB card),
shown and turned around in the desktop gallery and saved as .glb.

## Next, in order

Nothing queued: pick the next section.

## Optional extras, later

- **Training on a helper PC.** Training runs on the server only today, and needs about 16 GB
  of video memory. A helper PC with a big enough card (and ai-toolkit installed there) could
  take training jobs too: it fetches the dataset, trains, and sends the checkpoint samples and
  the finished LoRA back to the server. Pausing mid-training would stop the training, so the
  pause switch would warn first. Maybe lower-memory settings for cards under 16 GB.

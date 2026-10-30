# What's next

Done most recently: the Linux server (phase 6), helper PCs for Create, tidier Settings, the
one-command release (`scripts/release/ship.ps1`), picture tools (upscale and background removal,
from the gallery's picture viewer) and the Install ai-toolkit button in Train.

## Next, in order

1. **Chat with your documents**: attach PDFs and notes to a chat or a persona and ask about them.
2. **Share to Outpost from the phone**: send a picture or text from any app into chat or Create.
3. **Gallery search by description**: find pictures by what's in them, not only by prompt.
4. **Listening choice**: pick the speech-to-text model (e.g. Whisper large-v3-turbo) in Voice.
5. **3D models**: make a 3D model from a picture (Hunyuan3D or TRELLIS).

## Optional extras, later

- **Training on a helper PC.** Training runs on the server only today, and needs about 16 GB
  of video memory. A helper PC with a big enough card (and ai-toolkit installed there) could
  take training jobs too: it fetches the dataset, trains, and sends the checkpoint samples and
  the finished LoRA back to the server. Pausing mid-training would stop the training, so the
  pause switch would warn first. Maybe lower-memory settings for cards under 16 GB.

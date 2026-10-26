# Phase 5a live test: API page, MCP and webhooks

The gate: **another app draws one of your characters through Outpost's MCP, and a
webhook tells your receiver when it's done.**

Start Outpost as usual (`npm run dev`). You'll need an app key: **Connections › App keys
› New key** (copy the `op_...` it shows).

## 1. The API page

Open **API** in the left bar.

**Pass:** it lists where apps connect (this PC, plus your network and Tailscale if they're
on), a Hermes config and curl example with your first persona, the MCP address, a Claude
Desktop config with a real path to `outpost-gateway.exe`, Outpost's eight MCP tools, and
every endpoint grouped by area. Typing "train" in the finder narrows the list; opening an
endpoint shows a curl line to copy.

## 2. MCP from PowerShell (quick check)

```powershell
$key = "op_your_app_key"
$h = @{ Authorization = "Bearer $key" }
function Mcp($method, $params) {
  $body = @{ jsonrpc = "2.0"; id = 1; method = $method; params = $params } | ConvertTo-Json -Depth 6
  Invoke-RestMethod -Method Post -Uri http://127.0.0.1:7420/mcp -Headers $h -ContentType "application/json" -Body $body -TimeoutSec 120
}
(Mcp "tools/list" @{}).result.tools.name
(Mcp "tools/call" @{ name = "list_characters"; arguments = @{} }).result.content[0].text
(Mcp "tools/call" @{ name = "chat"; arguments = @{ message = "Say hi in five words" } }).result.content[0].text
$r = Mcp "tools/call" @{ name = "draw"; arguments = @{ prompt = "Kat at the beach" } }
$r.result.content | Select-Object type, text, mimeType
```

**Pass:** the tool names, your characters, a short reply, then (after the picture is drawn)
a text block with the prompt and an `image` block. The picture is in your gallery too.
If it took longer than 50 seconds (models loading), the text says the job is still
drawing: `(Mcp "tools/call" @{ name = "job_status"; arguments = @{ job_id = "<id>" } }).result.content`
fetches it.

## 3. Claude Desktop (the gate, if you use it)

1. Copy the **Claude Desktop config** from the API page, put your app key in
   `OUTPOST_KEY`, and add it to `claude_desktop_config.json` (Claude Desktop › Settings ›
   Developer › Edit config). Restart Claude Desktop.
2. Ask it: *"Use Outpost to draw Kat at the beach at sunset."*

**Pass:** Claude Desktop shows Outpost's tools, calls `draw`, and shows the picture.
(Hermes or another MCP app over HTTP works the same: URL `http://<PC>:7420/mcp`, header
`Authorization: Bearer op_...`.)

## 4. Webhooks

1. Start the test receiver in a second PowerShell window:
   `powershell -ExecutionPolicy Bypass -File .\scripts\webhooks\listen.ps1`
2. **Connections › Webhooks**: Send to `http://127.0.0.1:8099/`, tick all three, **Add
   webhook**. Copy the secret it shows, stop the receiver (Ctrl+C) and start it again
   with `-Secret whsec_...`.
3. **Send a test**: the receiver prints a `ping` with "signature OK"; the row says
   "Delivered (200)".
4. Draw something (Create, chat or MCP): the receiver prints `job.finished` with the job,
   its prompt and the picture's `/api/media/<id>`, "signature OK".
5. Stop the receiver and draw again: after about a minute the webhook row shows the
   failure ("Couldn't connect"), and Outpost carries on normally.

## What to send Claude

Anything that failed, and whether Claude Desktop (or Hermes) drew Kat.

# Write a keyword caption (.txt) next to every image in a folder, using a vision
# model through Outpost's API. This is a preview of Training Studio's auto-captioner
# (phase 4) and makes the training dataset for the ai-toolkit spike.
#
#   powershell -ExecutionPolicy Bypass -File .\scripts\spike\caption-folder.ps1 `
#       -Folder "C:\datasets\rex" -Trigger "rexdog" -Key "op_..." -Model "ornith-1.5:9b"
#
# -Key: an Outpost app key (Settings > App keys). -Model: any installed model with
# vision (the eye badge). Existing .txt files are kept unless -Overwrite is given.

param(
    [Parameter(Mandatory = $true)][string]$Folder,
    [Parameter(Mandatory = $true)][string]$Key,
    [string]$Model = "ornith-1.5:9b",
    [string]$Trigger = "",
    [string]$Gateway = "http://127.0.0.1:7420",
    [switch]$Overwrite
)
$ErrorActionPreference = "Stop"

$prompt = @"
Describe this image for training an image generation model.
Reply with ONE line of comma-separated keywords and short phrases, most important first:
main subject, appearance (hair, face, body, age), clothing, pose and expression, action,
setting and background, lighting, colours, camera angle and framing, art style or photo type.
No full sentences, no preamble, no quotes.
"@

$types = @{ ".png" = "image/png"; ".jpg" = "image/jpeg"; ".jpeg" = "image/jpeg"; ".webp" = "image/webp" }
$images = Get-ChildItem -LiteralPath $Folder -File | Where-Object { $types.ContainsKey($_.Extension.ToLower()) }
if (-not $images) { throw "No PNG, JPEG or WebP images in $Folder" }

$done = 0
foreach ($img in $images) {
    $txt = [IO.Path]::ChangeExtension($img.FullName, ".txt")
    if ((Test-Path -LiteralPath $txt) -and -not $Overwrite) {
        Write-Host "skip   $($img.Name) (has a caption)"
        continue
    }
    if ($img.Length -gt 10MB) {
        Write-Host "skip   $($img.Name) (over 10 MB)" -ForegroundColor Yellow
        continue
    }
    $b64 = [Convert]::ToBase64String([IO.File]::ReadAllBytes($img.FullName))
    $mime = $types[$img.Extension.ToLower()]
    $body = @{
        model       = $Model
        temperature = 0.2
        messages    = @(
            @{
                role    = "user"
                content = @(
                    @{ type = "text"; text = $prompt },
                    @{ type = "image_url"; image_url = @{ url = "data:$mime;base64,$b64" } }
                )
            }
        )
    } | ConvertTo-Json -Depth 10
    try {
        $resp = Invoke-RestMethod -Method Post -Uri "$Gateway/v1/chat/completions" `
            -Headers @{ Authorization = "Bearer $Key" } -ContentType "application/json; charset=utf-8" `
            -Body ([Text.Encoding]::UTF8.GetBytes($body)) -TimeoutSec 300
    } catch {
        # The gateway explains what went wrong in the body; show that, not just "(400)".
        $detail = if ($_.ErrorDetails -and $_.ErrorDetails.Message) { $_.ErrorDetails.Message } else { $_.Exception.Message }
        Write-Host "FAIL   $($img.Name): $detail" -ForegroundColor Red
        continue
    }
    $caption = [string]$resp.choices[0].message.content
    # Tidy: drop any reasoning block, keep one line, no trailing full stop.
    $caption = [regex]::Replace($caption, "(?s)<think>.*?</think>", "")
    $caption = ($caption -split "`r?`n" | Where-Object { $_.Trim() } | Select-Object -First 1)
    if (-not $caption) { Write-Host "FAIL   $($img.Name): empty caption" -ForegroundColor Red; continue }
    $caption = $caption.Trim().TrimEnd(".")
    if ($Trigger) { $caption = "$Trigger, $caption" }
    [IO.File]::WriteAllText($txt, $caption, (New-Object Text.UTF8Encoding($false)))
    $done++
    Write-Host "ok     $($img.Name): $caption"
}
Write-Host "Captioned $done image(s)." -ForegroundColor Green

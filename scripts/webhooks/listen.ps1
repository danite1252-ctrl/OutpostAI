# A tiny webhook receiver for testing Outpost's webhooks on this PC. It prints each
# message and checks its X-Outpost-Signature with the webhook's secret.
#
#   powershell -ExecutionPolicy Bypass -File .\scripts\webhooks\listen.ps1 -Secret whsec_...
#
# Then in Outpost: Connections > Webhooks > Send to  http://127.0.0.1:8099/  > Add.
# Ctrl+C to stop.

param(
    [string]$Secret = "",
    [int]$Port = 8099
)
$ErrorActionPreference = "Stop"

function Get-Signature([string]$key, [byte[]]$body) {
    $hmac = New-Object System.Security.Cryptography.HMACSHA256
    $hmac.Key = [Text.Encoding]::UTF8.GetBytes($key)
    ($hmac.ComputeHash($body) | ForEach-Object { $_.ToString("x2") }) -join ""
}

$listener = New-Object System.Net.HttpListener
$listener.Prefixes.Add("http://127.0.0.1:$Port/")
$listener.Start()
Write-Host "Listening on http://127.0.0.1:$Port/  (Ctrl+C to stop)" -ForegroundColor Yellow
if (-not $Secret) { Write-Host "No -Secret given: signatures won't be checked." -ForegroundColor DarkYellow }
try {
    while ($listener.IsListening) {
        $ctx = $listener.GetContext()
        $req = $ctx.Request
        $ms = New-Object System.IO.MemoryStream
        $req.InputStream.CopyTo($ms)
        $bytes = $ms.ToArray()
        $text = [Text.Encoding]::UTF8.GetString($bytes)
        $evt = $req.Headers["X-Outpost-Event"]
        $sig = $req.Headers["X-Outpost-Signature"]
        $verdict = "not checked"
        if ($Secret) {
            $verdict = if ($sig -eq "sha256=$(Get-Signature $Secret $bytes)") { "signature OK" } else { "SIGNATURE WRONG" }
        }
        Write-Host ""
        Write-Host "[$(Get-Date -Format HH:mm:ss)] $evt  ($verdict)" -ForegroundColor Green
        try { $text | ConvertFrom-Json | ConvertTo-Json -Depth 8 | Write-Host } catch { Write-Host $text }
        $ctx.Response.StatusCode = 200
        $ctx.Response.Close()
    }
} finally {
    $listener.Stop()
}

# Start the ai-toolkit UI at http://localhost:8675 (Ctrl+C to stop).
# Close Outpost's chat model first, or stop Ollama in Settings > Engines, so the
# trainer has the whole GPU.
param(
    [string]$Target = "C:\dev\ai-toolkit"
)
$ErrorActionPreference = "Stop"
if (-not (Test-Path (Join-Path $Target "venv\Scripts\python.exe"))) { throw "Run install-ai-toolkit.ps1 first." }
# The UI starts training jobs with the venv's Python.
$env:PATH = (Join-Path $Target "venv\Scripts") + ";" + $env:PATH
$env:VIRTUAL_ENV = Join-Path $Target "venv"
Set-Location (Join-Path $Target "ui")
Write-Host "Building and starting the ai-toolkit UI: http://localhost:8675" -ForegroundColor Green
# The README's documented command (builds the UI, then serves it).
npm run build_and_start

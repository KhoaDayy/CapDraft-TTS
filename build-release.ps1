param(
    [string]$Version = "1.1.2"
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
Set-Location $Root

if (-not (Get-Command uv -ErrorAction SilentlyContinue)) {
    throw "uv is required. Install it from https://docs.astral.sh/uv/"
}

uv sync --frozen --group dev
uv run --frozen pyinstaller --noconfirm --clean CapDraft-TTS.spec

$ReleaseDir = Join-Path $Root "release"
New-Item -ItemType Directory -Force $ReleaseDir | Out-Null
$Archive = Join-Path $ReleaseDir "CapDraft-TTS-v$Version-windows-x64.zip"
if (Test-Path $Archive) {
    Remove-Item -LiteralPath $Archive -Force
}
Compress-Archive -Path "dist\CapDraft-TTS\*" -DestinationPath $Archive -CompressionLevel Optimal

$Hash = (Get-FileHash -Algorithm SHA256 $Archive).Hash.ToLowerInvariant()
"$Hash  $(Split-Path -Leaf $Archive)" | Set-Content -Encoding ascii "$Archive.sha256"

Write-Host "Built: $Archive"
Write-Host "SHA256: $Hash"

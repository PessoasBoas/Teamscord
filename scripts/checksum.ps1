$ErrorActionPreference = "Stop"

$bundleDirectory = Join-Path (Get-Location) "src-tauri\target\release\bundle\nsis"
$tauriConfigPath = Join-Path (Get-Location) "src-tauri\tauri.conf.json"
$tauriConfig = Get-Content -LiteralPath $tauriConfigPath -Raw | ConvertFrom-Json
$installerName = "$($tauriConfig.productName)_$($tauriConfig.version)_x64-setup.exe"
$installerPath = Join-Path $bundleDirectory $installerName
if (-not (Test-Path -LiteralPath $installerPath -PathType Leaf)) {
  throw "O instalador NSIS esperado não foi encontrado: $installerPath"
}

$installer = Get-Item -LiteralPath $installerPath
$sha256 = [System.Security.Cryptography.SHA256]::Create()
try {
  $stream = [System.IO.File]::OpenRead($installer.FullName)
  try {
    $hashBytes = $sha256.ComputeHash($stream)
  } finally {
    $stream.Dispose()
  }
} finally {
  $sha256.Dispose()
}
$hash = ([System.BitConverter]::ToString($hashBytes)).Replace('-', '').ToLowerInvariant()
$checksumPath = "$($installer.FullName).sha256"
"$hash *$($installer.Name)" | Set-Content -LiteralPath $checksumPath -Encoding ascii
Write-Output "$($installer.Name): $hash"

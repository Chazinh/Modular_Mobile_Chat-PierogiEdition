# Starts the ModularChat server (Rust) on Windows.
#   .\start-server.ps1                 production: encrypted vault in .\data (or $env:MODULARCHAT_DATA), LAN access
#   .\start-server.ps1 -Sandbox        isolated copy of the vault for experiments (loopback only, port 3011)
# Before opening an existing vault the script takes an encrypted, verified snapshot of it.
param(
  [int]$Port = 0,
  [switch]$Lan,            # sandbox only: also listen on the LAN
  [switch]$Loopback,       # production behind the HTTPS proxy: do not expose Rust to the LAN
  [switch]$Sandbox,        # work on an isolated copy of the vault
  [switch]$ResetSandbox,   # with -Sandbox: back up and discard the sandbox vault, then copy again
  [switch]$LocalModels,    # sandbox only: also start the local memory models
  [switch]$NoUpstream,     # disable text generation (no provider is contacted)
  [switch]$Production,     # explicit production mode (the default)
  [string]$Upstream = '',  # OpenAI-compatible base URL when there is no providers.json
  [string]$DataDir = '',   # override the data directory
  [string]$Exe = ''        # use this server executable instead of the release build
)
$ErrorActionPreference = 'Stop'
if ($Sandbox -and $Production) { throw 'Choose either -Production or -Sandbox.' }
if ($Lan -and $Loopback) { throw 'Choose either -Lan or -Loopback.' }
if ($ResetSandbox -and -not $Sandbox) { throw '-ResetSandbox requires -Sandbox; the main vault is never discarded.' }

function Get-FileSha256([string]$Path) {
  $sha = [Security.Cryptography.SHA256]::Create()
  $stream = [IO.File]::OpenRead($Path)
  try { ([BitConverter]::ToString($sha.ComputeHash($stream))).Replace('-', '').ToLowerInvariant() }
  finally { $stream.Dispose(); $sha.Dispose() }
}

# The executable runs from a per-build staging copy so `cargo build` can replace the original while it runs.
$appHome = Join-Path $env:LOCALAPPDATA 'ModularChat'
$build = Join-Path $PSScriptRoot 'server-rust\target\release\modularchat-server.exe'
if ($Exe) { $server = (Resolve-Path -LiteralPath $Exe).Path } else {
  if (-not (Test-Path -LiteralPath $build)) { throw 'Build the server first: cargo build --release --manifest-path server-rust/Cargo.toml' }
  $hash = (Get-FileSha256 $build).Substring(0, 16)
  $stage = Join-Path $appHome ('server\' + $hash)
  New-Item -ItemType Directory -Path $stage -Force | Out-Null
  $server = Join-Path $stage 'modularchat-server.exe'
  if (-not (Test-Path -LiteralPath $server)) { Copy-Item -LiteralPath $build -Destination $server }
  if ((Get-FileSha256 $server) -ne (Get-FileSha256 $build)) { throw 'Staged executable differs from the build; rebuild and try again.' }
}

$mainData = [IO.Path]::GetFullPath($(if ($env:MODULARCHAT_DATA) { $env:MODULARCHAT_DATA } else { Join-Path $PSScriptRoot 'data' }))
$data = if ($DataDir) { [IO.Path]::GetFullPath($DataDir) } elseif ($Sandbox) { Join-Path $appHome 'sandbox-data' } else { $mainData }
if ($Sandbox -and ($data -eq $mainData -or $data.StartsWith($mainData + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase))) { throw 'The sandbox must live outside the main data directory.' }
if (-not $Port) { $Port = if ($Sandbox) { 3011 } else { 3000 } }
if ($Sandbox -and $Port -in 3000, 20481, 20482) { throw 'Protected port; use 3011 (or another free port) for the sandbox.' }

# Replace a previous ModularChat instance on the same port; never stop other programs.
$listener = Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue | Select-Object -First 1
if ($listener) {
  $process = Get-CimInstance Win32_Process -Filter ('ProcessId=' + $listener.OwningProcess)
  if ($process.Name -ne 'modularchat-server.exe') { throw "Port $Port is used by $($process.Name) (PID $($process.ProcessId)). Nothing was stopped." }
  Write-Host "Stopping the previous ModularChat server on port $Port (PID $($process.ProcessId))..."
  Stop-Process -Id $process.ProcessId -Force -ErrorAction SilentlyContinue
  $free = $false
  for ($i = 0; $i -lt 20; $i++) {
    Start-Sleep -Milliseconds 250
    if (-not (Get-NetTCPConnection -State Listen -LocalPort $Port -ErrorAction SilentlyContinue)) { $free = $true; break }
  }
  if (-not $free) { throw "Port $Port is still busy after stopping PID $($process.ProcessId)." }
}

New-Item -ItemType Directory -Path $data -Force | Out-Null
$vault = Join-Path $data 'modularchat.db.vault'
if (Test-Path -LiteralPath (Join-Path $data 'modularchat.db')) { throw 'A plain-text legacy database was found. Keep it safe; this launcher only opens the encrypted vault.' }
if ($Sandbox) {
  if ((Test-Path -LiteralPath $vault) -and $ResetSandbox) {
    if (Test-Path -LiteralPath (Join-Path $data 'modularchat.db.vault.lock')) { throw 'The sandbox may be open; stop it before resetting.' }
    $recovery = Join-Path $data ('backups\before-reset-' + (Get-Date -Format yyyyMMdd-HHmmss) + '-' + [guid]::NewGuid().ToString('N'))
    & $server snapshot $vault (Join-Path $recovery 'modularchat.db.vault')
    if ($LASTEXITCODE -ne 0) { throw 'Sandbox backup could not be verified; nothing was removed.' }
    foreach ($name in @('modularchat.db.vault', 'modularchat.db.key')) {
      $item = [IO.Path]::GetFullPath((Join-Path $data $name))
      if (-not $item.StartsWith($data + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'Invalid sandbox path.' }
      Remove-Item -LiteralPath $item
    }
  }
  if (-not (Test-Path -LiteralPath $vault) -and (Test-Path -LiteralPath (Join-Path $mainData 'modularchat.db.vault'))) {
    & $server snapshot (Join-Path $mainData 'modularchat.db.vault') $vault
    if ($LASTEXITCODE -ne 0) { throw 'Encrypted sandbox copy could not be verified.' }
    foreach ($extra in @('chats', 'route-validation.json', 'providers.json')) {
      $source = Join-Path $mainData $extra
      if (Test-Path -LiteralPath $source) { Copy-Item -LiteralPath $source -Destination $data -Recurse -Force }
    }
  }
} elseif (Test-Path -LiteralPath $vault) {
  $backup = Join-Path $data ('backups\start-' + (Get-Date -Format yyyyMMdd-HHmmss) + '-' + [guid]::NewGuid().ToString('N') + '\modularchat.db.vault')
  & $server snapshot $vault $backup
  if ($LASTEXITCODE -ne 0) { throw 'Encrypted backup could not be verified; the server was not started.' }
  $env:MC_RUST_BACKUP_FILE = $backup
}

$env:DATA_DIR = $data
$env:HOST = if ($Loopback -or ($Sandbox -and -not $Lan)) { '127.0.0.1' } else { '0.0.0.0' }
$env:PORT = "$Port"
$env:REQUIRE_PAIRING = '1'
$env:MC_ENCRYPTED_STORAGE = '1'
$env:MC_RUST_MODE = if ($Sandbox) { 'experimental' } else { 'production' }
$env:MC_RUST_VALIDATED = '2.7.2'   # handshake: production only opens a vault through this launcher
$env:MC_RUST_UPSTREAM_ENABLED = if ($NoUpstream) { '0' } else { '1' }
if ($Upstream) { $env:MC_UPSTREAM_URL = $Upstream.TrimEnd('/') }
$env:MC_LOCAL_MODELS_ENABLED = if ($LocalModels -or -not $Sandbox) { '1' } else { '0' }
$env:LOCAL_CHAT_PORT = if ($Sandbox) { '20581' } else { '20481' }
$env:LOCAL_EMBED_PORT = if ($Sandbox) { '20582' } else { '20482' }
Remove-Item Env:MC_TEST_LOCAL_URL -ErrorAction SilentlyContinue

Write-Host "ModularChat on port $Port. Vault: $vault"
Write-Host "On this PC: http://127.0.0.1:$Port"
if ($env:HOST -eq '0.0.0.0') {
  Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.IPAddress -notlike '127.*' -and $_.IPAddress -notlike '169.254.*' } |
    ForEach-Object { Write-Host "On your phone: http://$($_.IPAddress):$Port" }
}
Write-Host 'The pairing code appears below. Ctrl+C stops the server and saves the vault.'
& $server
exit $LASTEXITCODE

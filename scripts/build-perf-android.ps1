# Builds the MEASUREMENT copy of the Android app (build type "perf") and, if a phone is connected, installs it.
# It is a separate package (<applicationId>.perf) with its own empty storage, so it never touches the real app,
# and it has no FLAG_SECURE and no biometric gate, so screenshots and adb measurements work.
# Never distribute it. The APK stays under the isolated build folder and is not copied anywhere else.
#
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/build-perf-android.ps1 [-NoInstall] [-Local <folder>]
# Toolchain (JDK, Android SDK, Gradle home) comes from the local folder (default ModularChat-local in the user profile).
param(
  [switch]$NoInstall,
  [string]$Local = (Join-Path $env:USERPROFILE 'ModularChat-local')
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$client = Join-Path $project 'client'
$android = Join-Path $client 'android'
$jdk = Get-ChildItem (Join-Path $Local 'jdk') -Directory | Select-Object -First 1
if (-not $jdk) { throw "JDK not found under $Local\jdk" }
$build = Join-Path $Local 'build-isolated\android-build-perf'
$tools = Join-Path $Local 'build-isolated\tools'
$tmp = Join-Path $Local 'tmp'
New-Item -ItemType Directory -Force $tools, $tmp | Out-Null

# Same trick as the other isolated builds: build output outside the project (and outside the cloud-synced folder).
$init = Join-Path $tools 'android-isolated-perf.gradle'
$gradle = "allprojects {`n    layout.buildDirectory.set(new File('$($build.Replace('\','/'))', project.path.replace(':', '_')))`n}`n"
[IO.File]::WriteAllText($init, $gradle, (New-Object Text.UTF8Encoding $false))

$overrides = @{
  JAVA_HOME = $jdk.FullName
  Path = "$($jdk.FullName)\bin;$env:Path"
  GRADLE_USER_HOME = (Join-Path $Local 'gradle-home')
  TEMP = $tmp; TMP = $tmp
  ANDROID_HOME = (Join-Path $Local 'android-sdk')
}
$trust = Join-Path $android 'host-trust'
if (-not $env:MC_ANDROID_HOST_TRUST_DIR -and (Test-Path (Join-Path $trust 'xml\modularchat_host_security_config.xml'))) { $overrides.MC_ANDROID_HOST_TRUST_DIR = $trust }
$previous = @{}

try {
foreach ($name in $overrides.Keys) {
  $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
  [Environment]::SetEnvironmentVariable($name, $overrides[$name], 'Process')
}

# Native helpers finish before frontend sync or Gradle packaging. Rust exports JNI directly.
Write-Host '1. Android ARM64 Rust runtime'
& (Join-Path $PSScriptRoot 'build-android-autohost.ps1') -Local $Local
Write-Host '2. Android ARM64 llama-server executable'
& (Join-Path $PSScriptRoot 'build-android-llama.ps1') -Local $Local

Push-Location $client
try {
  Write-Host '3. Client build (vite)'
  & npm.cmd run build; if ($LASTEXITCODE) { throw 'vite build failed' }
  Write-Host '4. Capacitor sync'
  & npx.cmd cap sync android; if ($LASTEXITCODE) { throw 'cap sync failed' }
} finally { Pop-Location }

Push-Location $android
try {
  Write-Host '5. Gradle assemblePerf'
  & .\gradlew.bat --no-daemon --init-script $init assemblePerf
  if ($LASTEXITCODE) { throw "Gradle failed with code $LASTEXITCODE" }
} finally { Pop-Location }

$apk = Get-ChildItem $build -Recurse -Filter *perf*.apk -ErrorAction SilentlyContinue | Sort-Object LastWriteTime -Descending | Select-Object -First 1
if (-not $apk) { throw 'APK perf not found after the build.' }
Write-Host "APK: $($apk.FullName) ($([math]::Round($apk.Length / 1MB, 1)) MB)"

if (-not $NoInstall) {
  $adb = Join-Path $Local 'android-sdk\platform-tools\adb.exe'
  $devices = & $adb devices | Select-String -Pattern '\tdevice$'
  if (-not $devices) { Write-Host 'No phone connected by adb: build done, nothing installed (rerun with the phone connected, USB debugging on).'; return }
  & $adb install -r $apk.FullName
  if ($LASTEXITCODE) { throw 'adb install failed' }
  Write-Host 'Installed beside the real app as <applicationId>.perf.'
}
} finally {
  foreach ($name in $previous.Keys) {
    if ($null -eq $previous[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
    else { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
  }
}

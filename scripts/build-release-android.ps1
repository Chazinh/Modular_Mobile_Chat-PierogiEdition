# Current, signed AutoHost release. Never substitutes a perf/old APK or changes the signing key.
# -NoInstall still verifies the complete APK and compares its certificate with the installed app.
[CmdletBinding()]
param(
    [string]$Local = (Join-Path $env:USERPROFILE 'ModularChat-local'),
    [ValidatePattern('^[A-Za-z0-9_.:-]+$')][string]$Serial = '',
    [switch]$NoInstall,
    [string]$PrivateBuildProperties = '',
    [string]$SigningProperties = ''
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$Local = [IO.Path]::GetFullPath($Local)
if ($Local.Equals($project, [StringComparison]::OrdinalIgnoreCase) -or
    $Local.StartsWith($project.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Local must be outside the repository.'
}
$client = Join-Path $project 'client'
$android = Join-Path $client 'android'
$build = Join-Path $Local 'build-isolated\android-build-autohost-release'
$logs = Join-Path $build 'logs'
$tmp = Join-Path $build 'tmp'
$sdk = Join-Path $Local 'android-sdk'
$tools = Join-Path $sdk 'build-tools\36.0.0'
$adb = Join-Path $sdk 'platform-tools\adb.exe'
$readelf = Join-Path $sdk 'ndk\28.2.13676358\toolchains\llvm\prebuilt\windows-x86_64\bin\llvm-readelf.exe'
$jdk = Get-ChildItem -LiteralPath (Join-Path $Local 'jdk') -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName 'bin\java.exe') } | Select-Object -First 1
if (-not $jdk -or (Get-Content -LiteralPath (Join-Path $jdk.FullName 'release') -Raw) -notmatch 'JAVA_VERSION="21\.') {
    throw 'The cached JDK 21 is required.'
}
if (-not $PrivateBuildProperties) { $PrivateBuildProperties = Join-Path $android 'private-build.properties' }
if (-not $SigningProperties) { $SigningProperties = Join-Path $Local 'signing\keystore.properties' }
$PrivateBuildProperties = [IO.Path]::GetFullPath($PrivateBuildProperties)
$SigningProperties = [IO.Path]::GetFullPath($SigningProperties)
foreach ($required in @($PrivateBuildProperties, $SigningProperties, $adb, $readelf,
    (Join-Path $tools 'apksigner.bat'), (Join-Path $tools 'aapt2.exe'), (Join-Path $tools 'aapt.exe'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required release input missing: $required" }
}
New-Item -ItemType Directory -Path $build, $logs, $tmp -Force | Out-Null
$utf8 = New-Object Text.UTF8Encoding $false
$steps = New-Object Collections.Generic.List[object]
$lock = [IO.File]::Open((Join-Path $build 'release-build.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)

function Invoke-ReleaseTool {
    param([string]$Executable, [string[]]$ToolArgs, [string]$Name,
        [string]$Directory = $project, [int]$TimeoutSeconds = 60)
    $quoted = foreach ($arg in (@($Executable) + $ToolArgs)) {
        if ($arg -match '["\r\n]') { throw 'Unsupported quote/newline in tool argument.' }
        '"' + ($arg -replace '(\\+)$', '$1$1') + '"'
    }
    $arguments = ($quoted | Select-Object -Skip 1) -join ' '
    if ([IO.Path]::GetExtension($Executable) -in @('.cmd', '.bat')) {
        foreach ($arg in (@($Executable) + $ToolArgs)) {
            if ($arg -match '[%!^&|<>]') { throw 'Unsupported shell metacharacter in batch argument.' }
        }
        $arguments = '/d /s /c "' + ($quoted -join ' ') + '"'
        $Executable = $env:ComSpec
    }
    $stdout = Join-Path $logs "$Name.stdout.log"
    $stderr = Join-Path $logs "$Name.stderr.log"
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $Executable -ArgumentList $arguments -WorkingDirectory $Directory `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Holding the handle avoids a null/false exit code in Windows PowerShell 5.1.
    [void]$process.Handle
    try {
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            throw "$Name exceeded $TimeoutSeconds seconds; its process $($process.Id) may still be running. Do not start a duplicate."
        }
        $process.WaitForExit()
        $result = [ordered]@{ name = $Name; exitCode = $process.ExitCode
            seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3); stdout = $stdout; stderr = $stderr }
        $steps.Add($result)
        if ($process.ExitCode -ne 0) { throw "$Name failed (exit $($process.ExitCode)). Logs: $stdout ; $stderr" }
        Write-Host "$Name exit=0 ($($result.seconds)s)"
        return (Get-Content -LiteralPath $stdout -Raw)
    } finally { $timer.Stop(); $process.Dispose() }
}

function Read-BuildIdentity {
    $text = Get-Content -LiteralPath $PrivateBuildProperties -Raw
    $identity = @{}
    foreach ($key in @('applicationId', 'versionCode', 'versionName')) {
        $matches = [regex]::Matches($text, "(?m)^\s*$key\s*=\s*([^\r\n]+)")
        if ($matches.Count -ne 1) { throw "Expected one $key in the private build properties." }
        $identity[$key] = $matches[0].Groups[1].Value.Trim()
    }
    if ($identity.applicationId -notmatch '^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$' -or
        $identity.applicationId.EndsWith('.perf') -or $identity.versionCode -notmatch '^\d+$' -or
        $identity.versionName -notmatch '^\d+\.\d+\.\d+$') { throw 'Invalid release identity; perf is never a release.' }
    $identity.versionCode = [int]$identity.versionCode
    return $identity
}

function Get-InstalledVersion([string]$Name) {
    $package = Invoke-ReleaseTool $adb @('-s', $Serial, 'shell', 'dumpsys', 'package', $identity.applicationId) $Name
    if ($package -notmatch 'versionCode=(\d+)\b') { throw 'The existing package is unavailable; refusing a fresh install.' }
    return [int]$Matches[1]
}

function Get-InstalledCertificate([string]$Name) {
    $paths = Invoke-ReleaseTool $adb @('-s', $Serial, 'shell', 'pm', 'path', $identity.applicationId) "$Name-path"
    $base = @($paths -split '\r?\n' | Where-Object { $_ -match '^package:/data/app/.+/base\.apk$' })
    if ($base.Count -ne 1) { throw 'Expected exactly one installed base APK.' }
    $installedApk = Join-Path $tmp "$Name-base.apk"
    [void](Invoke-ReleaseTool $adb @('-s', $Serial, 'pull', $base[0].Substring(8), $installedApk) "$Name-pull" -TimeoutSeconds 120)
    return (Get-ApkCertificate $installedApk "$Name-cert")
}

function Get-ApkCertificate([string]$Apk, [string]$Name) {
    $result = Invoke-ReleaseTool (Join-Path $tools 'apksigner.bat') @('verify', '--verbose', '--print-certs', $Apk) $Name
    $certs = [regex]::Matches($result, '(?m)^Signer #\d+ certificate SHA-256 digest: ([0-9a-fA-F]{64})\s*$')
    if ($certs.Count -ne 1 -or $result -notmatch 'Number of signers: 1\b') { throw 'Exactly one verified signing certificate is required.' }
    return $certs[0].Groups[1].Value.ToUpperInvariant()
}

function Assert-SourceSnapshot {
    foreach ($file in $sourceHashes) {
        if (-not (Test-Path -LiteralPath $file.path -PathType Leaf) -or
            (Get-FileHash -LiteralPath $file.path -Algorithm SHA256).Hash -ne $file.sha256) {
            throw "Native/release source changed or disappeared during the build: $($file.path)"
        }
    }
}

function Get-DexDefinitions([byte[]]$Bytes) {
    if ($Bytes.Length -lt 112 -or [Text.Encoding]::ASCII.GetString($Bytes, 0, 4) -ne "dex`n") { throw 'Invalid DEX header.' }
    $stringCount = [BitConverter]::ToUInt32($Bytes, 56); $stringOffset = [BitConverter]::ToUInt32($Bytes, 60)
    $typeCount = [BitConverter]::ToUInt32($Bytes, 64); $typeOffset = [BitConverter]::ToUInt32($Bytes, 68)
    $classCount = [BitConverter]::ToUInt32($Bytes, 96); $classOffset = [BitConverter]::ToUInt32($Bytes, 100)
    foreach ($table in @(@($stringOffset, $stringCount, 4), @($typeOffset, $typeCount, 4), @($classOffset, $classCount, 32))) {
        if ([long]$table[0] + [long]$table[1] * $table[2] -gt $Bytes.Length) { throw 'DEX table exceeds file size.' }
    }
    for ($i = 0; $i -lt $classCount; $i++) {
        $type = [BitConverter]::ToUInt32($Bytes, ($classOffset + $i * 32))
        if ($type -ge $typeCount) { throw 'Invalid DEX class type index.' }
        $descriptor = [BitConverter]::ToUInt32($Bytes, ($typeOffset + $type * 4))
        if ($descriptor -ge $stringCount) { throw 'Invalid DEX descriptor index.' }
        $offset = [BitConverter]::ToUInt32($Bytes, ($stringOffset + $descriptor * 4))
        $uleb = 0
        do {
            if ($offset -ge $Bytes.Length -or ++$uleb -gt 5) { throw 'Invalid DEX string length.' }
            $value = $Bytes[$offset]; $offset++
        } while ($value -band 128)
        $end = $offset
        while ($end -lt $Bytes.Length -and $Bytes[$end] -ne 0) { $end++ }
        if ($end -eq $Bytes.Length) { throw 'Unterminated DEX descriptor.' }
        [Text.Encoding]::UTF8.GetString($Bytes, $offset, $end - $offset)
    }
}

$previous = @{}
try {
    $overrides = @{
        JAVA_HOME = $jdk.FullName; Path = "$($jdk.FullName)\bin;$env:Path"
        GRADLE_USER_HOME = (Join-Path $Local 'gradle-home'); TEMP = $tmp; TMP = $tmp
        ANDROID_HOME = $sdk; ANDROID_SDK_ROOT = $sdk
        MC_KEYSTORE_PROPS = $SigningProperties; MC_ANDROID_PRIVATE_PROPS = $PrivateBuildProperties
    }
    foreach ($key in $overrides.Keys) {
        $previous[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $overrides[$key], 'Process')
    }
    $identity = Read-BuildIdentity
    if ((Invoke-ReleaseTool $adb @('-s', $Serial, 'get-state') 'adb-state').Trim() -ne 'device') { throw 'The selected device is not ready.' }
    $installedCode = Get-InstalledVersion 'installed-before'
    if ($identity.versionCode -le $installedCode) {
        $text = Get-Content -LiteralPath $PrivateBuildProperties -Raw
        $text = [regex]::Replace($text, '(?m)^(\s*versionCode\s*=\s*)\d+', '${1}' + ($installedCode + 1))
        [IO.File]::WriteAllText($PrivateBuildProperties, $text, $utf8)
        $identity = Read-BuildIdentity
    }
    $installedCertificate = Get-InstalledCertificate 'installed-before'
    $sourcePaths = @('client/android/app/build.gradle', 'client/android/app/src/main/AndroidManifest.xml',
        'server-rust/src/android_jni.rs', 'server-rust/src/privacy.rs', 'server-rust/src/local.rs',
        'scripts/build-android-autohost.ps1', 'scripts/build-android-llama.ps1',
        'client/android/app/src/main/res/values/mc_perf.xml')
    foreach ($name in @('MainActivity', 'LocalHostPlugin', 'LocalHostService', 'NativeHost', 'NativeHostToken', 'NativeVaultKey')) {
        $sourcePaths += "client/android/app/src/main/java/io/github/modularchat/pierogiedition/$name.java"
    }
    $sourceHashes = @($sourcePaths | ForEach-Object {
        $path = Join-Path $project $_
        if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Restored source missing: $path" }
        [ordered]@{ path = $path; sha256 = (Get-FileHash -LiteralPath $path -Algorithm SHA256).Hash }
    })
    $activity = Get-Content -LiteralPath (Join-Path $android 'app/src/main/java/io/github/modularchat/pierogiedition/MainActivity.java') -Raw
    if ($activity -notmatch '(?m)^\s*if \(!perf\) getWindow\(\)\.addFlags\(WindowManager.LayoutParams.FLAG_SECURE\);' -or
        $activity -notmatch 'getBoolean\(R.bool.mc_perf\) && getPackageName\(\).endsWith\("\.perf"\)' -or
        $activity -match '(?m)^\s*perf\s*=\s*true\s*;' -or
        $activity -match 'setWebContentsDebuggingEnabled\(true\)' -or
        $activity -notmatch 'ApplicationInfo.FLAG_DEBUGGABLE' -or
        $activity -notmatch 'new BiometricPrompt' -or
        $activity -notmatch 'registerPlugin\(LocalHostPlugin.class\)') { throw 'Release security/plugin integration missing or bypassed.' }
    $shell = [Diagnostics.Process]::GetCurrentProcess().MainModule.FileName
    [void](Invoke-ReleaseTool $shell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        (Join-Path $PSScriptRoot 'build-android-autohost.ps1'), '-Local', $Local) 'rust-arm64' -TimeoutSeconds 900)
    [void](Invoke-ReleaseTool $shell @('-NoProfile', '-ExecutionPolicy', 'Bypass', '-File',
        (Join-Path $PSScriptRoot 'build-android-llama.ps1'), '-Local', $Local) 'llama-arm64' -TimeoutSeconds 1500)
    Assert-SourceSnapshot
    [void](Invoke-ReleaseTool (Get-Command npm.cmd -CommandType Application).Source @('run', 'build') 'vite-build' $client 240)
    [void](Invoke-ReleaseTool (Get-Command npx.cmd -CommandType Application).Source @('--no-install', 'cap', 'sync', 'android') 'capacitor-sync' $client 180)
    $init = Join-Path $build 'isolated-release.gradle'
    $gradleRoot = $build.Replace('\', '/').Replace("'", "\\'")
    [IO.File]::WriteAllText($init, "allprojects { layout.buildDirectory.set(new File('$gradleRoot', project.path.replace(':', '_'))) }", $utf8)
    [void](Invoke-ReleaseTool (Join-Path $android 'gradlew.bat') @('--no-daemon', '--console=plain', '--init-script',
        $init, ':app:assembleRelease') 'gradle-release' $android 1200)
    Assert-SourceSnapshot
    $apk = Join-Path $build '_app\outputs\apk\release\app-release.apk'
    if (-not (Test-Path -LiteralPath $apk -PathType Leaf)) { throw 'The current signed release output was not produced.' }
    $certificate = Get-ApkCertificate $apk 'release-cert'
    if ($certificate -ne $installedCertificate) { throw 'SIGNATURE MISMATCH. No install, uninstall, data clear or new key is allowed.' }
    $badging = Invoke-ReleaseTool (Join-Path $tools 'aapt.exe') @('dump', 'badging', $apk) 'release-badging'
    if ($badging -notmatch ("package: name='" + [regex]::Escape($identity.applicationId) + "' versionCode='" + $identity.versionCode +
        "' versionName='" + [regex]::Escape($identity.versionName) + "'") -or $badging -notmatch "native-code: 'arm64-v8a'\s*(\r?\n|$)" -or
        $badging -notmatch "sdkVersion:'24'" -or $badging -notmatch "targetSdkVersion:'36'") { throw 'Release package/version/ABI/SDK mismatch.' }
    $manifest = Invoke-ReleaseTool (Join-Path $tools 'aapt2.exe') @('dump', 'xmltree', $apk, '--file', 'AndroidManifest.xml') 'release-manifest'
    if ($manifest -notmatch 'android:extractNativeLibs[^\r\n]*=(?:true|[^\r\n]*0xffffffff)' -or
        $manifest -notmatch 'android:allowBackup[^\r\n]*=(?:false|[^\r\n]*0x0\b)' -or
        $manifest -match 'android:debuggable[^\r\n]*=(?:true|[^\r\n]*0xffffffff)') { throw 'Release native extraction/backup/debug settings invalid.' }
    $service = @([regex]::Matches($manifest, '(?ms)^\s*E: service\b.*?(?=^\s*E: (?:service|provider|activity|activity-alias|uses-permission)\b|\z)') |
        Where-Object { $_.Value -match 'io.github.modularchat.pierogiedition.LocalHostService' })
    if ($service.Count -ne 1 -or $service[0].Value -notmatch 'android:exported[^\r\n]*=(?:false|[^\r\n]*0x0\b)' -or
        $service[0].Value -notmatch 'android:foregroundServiceType[^\r\n]*0x40000000' -or
        $manifest -notmatch 'android.permission.FOREGROUND_SERVICE_SPECIAL_USE') { throw 'LocalHost foreground-service manifest invalid.' }
    $resources = Invoke-ReleaseTool (Join-Path $tools 'aapt2.exe') @('dump', 'resources', $apk) 'release-resources'
    if ($resources -notmatch 'bool/mc_perf\r?\n\s*\(\) false\b') { throw 'Release must package mc_perf=false.' }
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($apk)
    $native = New-Object Collections.Generic.List[object]
    $defined = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::Ordinal)
    $webFiles = 0
    try {
        $nativeNames = @('libmodularchat_server.so', 'libllama_server.so')
        $nativeEntries = @($zip.Entries | Where-Object { $_.FullName -match '^lib/.+/.+\.so$' })
        if ($nativeEntries.Count -ne 2) { throw 'Expected exactly the Rust/JNI runtime and the real llama ARM64 executable.' }
        foreach ($name in $nativeNames) {
            $entries = @($nativeEntries | Where-Object { $_.FullName -eq "lib/arm64-v8a/$name" })
            if ($entries.Count -ne 1) { throw "ARM64 artifact missing or duplicated: $name" }
            $file = Join-Path $tmp $name
            [IO.Compression.ZipFileExtensions]::ExtractToFile($entries[0], $file, $true)
            $built = Join-Path $build "_app\intermediates\stripped_native_libs\release\stripReleaseDebugSymbols\out\lib\arm64-v8a\$name"
            $hash = (Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash
            if (-not (Test-Path -LiteralPath $built) -or (Get-FileHash -LiteralPath $built -Algorithm SHA256).Hash -ne $hash) {
                throw "APK native artifact differs from this Gradle build: $name"
            }
            $elf = Invoke-ReleaseTool $readelf @('-h', '-lW', '-d', '--dyn-syms', $file) "elf-$name"
            if ($elf -notmatch 'Machine:\s+AArch64' -or $elf -notmatch 'Type:\s+DYN') { throw "Invalid ARM64 ELF: $name" }
            $loads = @($elf -split '\r?\n' | Where-Object { $_ -match '^\s*LOAD\s' })
            if (-not $loads.Count) { throw "Missing ELF LOAD segments: $name" }
            foreach ($segment in $loads) {
                if ($segment -notmatch '(0x[0-9a-f]+)\s*$' -or [Convert]::ToInt64($Matches[1].Substring(2), 16) -lt 16384) {
                    throw "ELF LOAD alignment below 16 KiB: $name"
                }
            }
            $needed = @([regex]::Matches($elf, '\(NEEDED\).*?\[([^\]]+)\]') | ForEach-Object { $_.Groups[1].Value })
            foreach ($dependency in $needed) {
                if ($dependency -notin @('libc.so', 'libdl.so', 'libm.so', 'liblog.so', 'libmodularchat_server.so')) {
                    throw "Unpackaged ELF dependency $dependency in $name"
                }
            }
            if ($name -eq 'libmodularchat_server.so') {
                foreach ($symbol in @('mc_android_start', 'mc_android_stop', 'mc_android_status', 'mc_android_last_error')) {
                    if ($elf -notmatch "\b$symbol\b") { throw "Rust C export missing: $symbol" }
                }
                foreach ($symbol in @('nativeStart', 'nativeStop', 'nativeStatus', 'nativeLastError')) {
                    if ($elf -notmatch "\bJava_io_github_modularchat_pierogiedition_NativeHost_$symbol\b") { throw "JNI export missing: $symbol" }
                }
            } else {
                $pinned = Get-Content -LiteralPath (Join-Path $Local 'build-isolated\android-autohost\llama-build-manifest.json') -Raw | ConvertFrom-Json
                $source = Join-Path $android 'app\src\main\jniLibs\arm64-v8a\libllama_server.so'
                # AGP applies --strip-unneeded even to a stripped PIE. Reproduce its exact output;
                # comparing only the original hash would reject a valid current release.
                $expected = Join-Path $tmp 'llama-release-stripped.so'
                $strip = Join-Path (Split-Path -Parent $readelf) 'llvm-strip.exe'
                [void](Invoke-ReleaseTool $strip @('--strip-unneeded', '-o', $expected, $source) 'llama-release-strip')
                if ($hash -ne (Get-FileHash -LiteralPath $expected -Algorithm SHA256).Hash -or
                    $pinned.source.commit -ne '318a471a6295e53bfce7ca96ae0cf26071cbdd6e' -or
                    $elf -notmatch 'Entry point address:\s+0x[1-9a-f][0-9a-f]*' -or $elf -notmatch 'INTERP\s' -or
                    @($needed | Where-Object { $_ -notin @('libc.so', 'libdl.so', 'libm.so') }).Count) { throw 'The pinned real llama PIE executable is required.' }
            }
            $native.Add([ordered]@{ name = $name; bytes = (Get-Item -LiteralPath $file).Length; sha256 = $hash; loadAlignmentMin = 16384; needed = $needed })
        }
        foreach ($entry in ($zip.Entries | Where-Object { $_.FullName -match '^classes\d*\.dex$' })) {
            $stream = $entry.Open(); $memory = New-Object IO.MemoryStream
            try { $stream.CopyTo($memory); foreach ($definition in (Get-DexDefinitions $memory.ToArray())) { [void]$defined.Add($definition) } }
            finally { $stream.Dispose(); $memory.Dispose() }
        }
        $classes = @('MainActivity', 'LocalHostPlugin', 'LocalHostService', 'NativeHost', 'NativeHostToken', 'NativeVaultKey')
        foreach ($name in $classes) {
            if (-not $defined.Contains("Lio/github/modularchat/pierogiedition/$name;")) { throw "Actual DEX class definition missing: $name" }
        }
        $sha = [Security.Cryptography.SHA256]::Create()
        try {
            foreach ($file in (Get-ChildItem -LiteralPath (Join-Path $client 'dist') -Recurse -File)) {
                $relative = $file.FullName.Substring((Join-Path $client 'dist').Length + 1).Replace('\', '/')
                $entries = @($zip.Entries | Where-Object { $_.FullName -eq "assets/public/$relative" })
                if ($entries.Count -ne 1) { throw "Current web asset missing or duplicated: $relative" }
                $stream = $entries[0].Open()
                try { $hash = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') } finally { $stream.Dispose() }
                if ($hash -ne (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash) { throw "APK web asset differs from current Vite output: $relative" }
                $webFiles++
            }
        } finally { $sha.Dispose() }
        if (-not $webFiles) { throw 'Empty web bundle.' }
    } finally { $zip.Dispose() }
    Assert-SourceSnapshot
    $latestCode = Get-InstalledVersion 'installed-final'
    $latestCertificate = Get-InstalledCertificate 'installed-final'
    if ($latestCertificate -ne $certificate) { throw 'SIGNATURE MISMATCH. Installation stopped without touching data.' }
    if ($identity.versionCode -le $latestCode) { throw 'Installed version advanced during the build. Rebuild with max+1; never downgrade.' }
    $result = [ordered]@{ status = 'verified-noinstall'; at = [DateTimeOffset]::Now.ToString('o'); serial = $Serial
        applicationId = $identity.applicationId; versionCode = $identity.versionCode; versionName = $identity.versionName
        apk = $apk; bytes = (Get-Item -LiteralPath $apk).Length; sha256 = (Get-FileHash -LiteralPath $apk -Algorithm SHA256).Hash
        certificateSha256 = $certificate; installedCertificateSha256 = $latestCertificate; previousVersionCode = $latestCode
        native = @($native.ToArray()); dexDefinitions = $classes; currentWebAssetsVerified = $webFiles; perf = $false
        sources = $sourceHashes; steps = @($steps.ToArray()) }
    $validation = Join-Path $build 'release-validation.json'
    [IO.File]::WriteAllText($validation, ($result | ConvertTo-Json -Depth 8), $utf8)
    Write-Host "VERIFIED APK: $apk ($($result.bytes) bytes); $($identity.versionCode) / $($identity.versionName); SHA256 $($result.sha256)"
    Write-Host "Signing certificate matches existing app: $certificate"
    if (-not $NoInstall) {
        $installed = Invoke-ReleaseTool $adb @('-s', $Serial, 'install', '-r', $apk) 'adb-install' -TimeoutSeconds 180
        if ($installed -notmatch '(?m)^Success\s*$') { throw 'adb exit=0 did not return literal Success.' }
        $package = Invoke-ReleaseTool $adb @('-s', $Serial, 'shell', 'dumpsys', 'package', $identity.applicationId) 'installed-after'
        if ($package -notmatch ("versionCode=" + $identity.versionCode + '\b') -or
            $package -notmatch ("versionName=" + [regex]::Escape($identity.versionName) + '\s') -or
            $package -notmatch 'primaryCpuAbi=arm64-v8a' -or $package -match '\bDEBUGGABLE\b') { throw 'Post-install identity/ABI/release verification failed.' }
        $result.status = 'installed'; $result.installResult = $installed.Trim(); $result.steps = @($steps.ToArray())
        [IO.File]::WriteAllText($validation, ($result | ConvertTo-Json -Depth 8), $utf8)
        Write-Host 'Success: signed release installed with -r, existing application data retained.'
    }
} finally {
    foreach ($key in $previous.Keys) {
        if ($null -eq $previous[$key]) { Remove-Item -LiteralPath "Env:$key" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($key, $previous[$key], 'Process') }
    }
    $lock.Dispose()
}

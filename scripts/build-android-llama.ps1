# Pinned, CPU-only Android executable. This never builds/installs an APK or downloads a model.
# Optional -Serial validates the packaged executable in a unique /data/local/tmp directory only.
[CmdletBinding()]
param(
    [string]$Local = (Join-Path $env:USERPROFILE 'RoleplayChat-local'),
    [ValidateRange(1, 32)][int]$Jobs = [Math]::Min(6, [Environment]::ProcessorCount),
    [ValidateRange(60, 1500)][int]$BuildTimeoutSeconds = 1200,
    [ValidatePattern('^[A-Za-z0-9_.:-]*$')][string]$Serial = ''
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$Local = [IO.Path]::GetFullPath($Local)
if ($Local.Equals($project, [StringComparison]::OrdinalIgnoreCase) -or
    $Local.StartsWith($project.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Local must be outside the repository; source and build caches are never vendored.'
}
$commit = '318a471a6295e53bfce7ca96ae0cf26071cbdd6e'
$archiveSha256 = 'E05190027B49416F5CCC42974A6F351AB912561658F43A69774EB371B8169DED'
$url = "https://codeload.github.com/ggml-org/llama.cpp/zip/$commit"
$work = Join-Path $Local 'build-isolated\android-autohost'
$sourceRoot = Join-Path $work 'llama-source'
$source = Join-Path $sourceRoot $commit
$archive = Join-Path $sourceRoot "$commit.zip"
$build = Join-Path $work "llama-build\$commit\arm64-v8a"
$logs = Join-Path $work 'llama-logs'
$tmp = Join-Path $work 'tmp'
$manifestPath = Join-Path $work 'llama-build-manifest.json'
$sdk = Join-Path $Local 'android-sdk'
$ndk = Join-Path $sdk 'ndk\28.2.13676358'
$ndkBin = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$cmake = Join-Path $sdk 'cmake\3.22.1\bin\cmake.exe'
$ninja = Join-Path $sdk 'cmake\3.22.1\bin\ninja.exe'
$readelf = Join-Path $ndkBin 'llvm-readelf.exe'
$binary = Join-Path $build 'bin\llama-server'
$packaged = Join-Path $project 'client\android\app\src\main\jniLibs\arm64-v8a\libllama_server.so'
$utf8 = New-Object Text.UTF8Encoding $false

function Invoke-LlamaTool {
    param([string]$Executable, [string[]]$ToolArgs, [string]$Name, [int]$TimeoutSeconds = 60)
    # Start-Process joins ArgumentList on Windows; quote each complete argument, including -D values.
    $quoted = foreach ($arg in $ToolArgs) {
        if ($arg -match '["\r\n]') { throw 'Unsupported quote/newline in tool argument.' }
        '"' + ($arg -replace '(\\+)$', '$1$1') + '"'
    }
    $stdout = Join-Path $logs "$Name.stdout.log"
    $stderr = Join-Path $logs "$Name.stderr.log"
    $timer = [Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $Executable -ArgumentList ($quoted -join ' ') -WorkingDirectory $work `
        -WindowStyle Hidden -PassThru -RedirectStandardOutput $stdout -RedirectStandardError $stderr
    # Windows PowerShell 5.1 needs an open handle to retain the real exit code after WaitForExit.
    [void]$process.Handle
    try {
        if (-not $process.WaitForExit($TimeoutSeconds * 1000)) {
            # End this build's own process tree, never another build or local service.
            $stop = Start-Process -FilePath (Join-Path $env:SystemRoot 'System32\taskkill.exe') `
                -ArgumentList @('/PID', [string]$process.Id, '/T', '/F') -WindowStyle Hidden -PassThru
            if (-not $stop.WaitForExit(10000)) { throw "$Name timed out; process $($process.Id) may still be running. Do not start a duplicate." }
            $stop.Dispose()
            throw "$Name exceeded $TimeoutSeconds seconds. Logs: $stdout ; $stderr"
        }
        $process.WaitForExit() # drain redirected streams after the bounded wait
        $result = [ordered]@{
            executable = $Executable; arguments = $ToolArgs; exitCode = $process.ExitCode
            elapsedSeconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
            stdout = $stdout; stderr = $stderr
        }
        if ($process.ExitCode -ne 0) {
            foreach ($log in @($stderr, $stdout)) { Get-Content -LiteralPath $log -Tail 12 | Write-Host }
            throw "$Name failed (exit $($process.ExitCode)). Logs: $stdout ; $stderr"
        }
        Write-Host "$Name exit=0 ($($result.elapsedSeconds)s)"
        return $result
    } finally { $timer.Stop(); $process.Dispose() }
}

function Assert-IsolatedPath([string]$Path, [string]$Parent) {
    if (-not [IO.Path]::GetFullPath($Path).StartsWith([IO.Path]::GetFullPath($Parent).TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase)) {
        throw "Path escaped isolated directory: $Path"
    }
}

function Test-PinnedSource {
    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $zip = [IO.Compression.ZipFile]::OpenRead($archive)
    $sha = [Security.Cryptography.SHA256]::Create()
    $expected = New-Object 'Collections.Generic.HashSet[string]' ([StringComparer]::OrdinalIgnoreCase)
    $fingerprint = New-Object Text.StringBuilder
    $bytes = [long]0
    try {
        $prefix = "llama.cpp-$commit/"
        foreach ($entry in $zip.Entries) {
            if (-not $entry.FullName.StartsWith($prefix, [StringComparison]::Ordinal)) { throw 'Archive root does not match pinned commit.' }
            if ($entry.FullName.EndsWith('/')) { continue }
            $relative = $entry.FullName.Substring($prefix.Length)
            $file = Join-Path $source $relative
            Assert-IsolatedPath $file $source
            if (-not $expected.Add([IO.Path]::GetFullPath($file))) { throw "Duplicate source entry: $relative" }
            if (-not (Test-Path -LiteralPath $file -PathType Leaf)) { throw "Pinned source file missing: $relative" }
            $stream = $entry.Open()
            try { $hash = [BitConverter]::ToString($sha.ComputeHash($stream)).Replace('-', '') } finally { $stream.Dispose() }
            if ((Get-FileHash -LiteralPath $file -Algorithm SHA256).Hash -ne $hash) { throw "Pinned source cache changed: $relative" }
            [void]$fingerprint.Append($relative).Append(':').Append($hash).Append("`n")
            $bytes += $entry.Length
        }
        foreach ($file in (Get-ChildItem -LiteralPath $source -Recurse -File)) {
            if (-not $expected.Contains($file.FullName)) { throw "Unexpected file in pinned source cache: $($file.FullName)" }
        }
        return [ordered]@{
            files = $expected.Count; bytes = $bytes
            treeSha256 = [BitConverter]::ToString($sha.ComputeHash($utf8.GetBytes($fingerprint.ToString()))).Replace('-', '')
            verifiedAgainstArchive = $true
        }
    } finally { $sha.Dispose(); $zip.Dispose() }
}

foreach ($required in @($cmake, $ninja, $readelf, (Join-Path $ndkBin 'clang.exe'), (Join-Path $ndk 'build\cmake\android.toolchain.cmake'))) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { throw "Required installed official toolchain missing: $required" }
}
if ((Get-Content -LiteralPath (Join-Path $ndk 'source.properties') -Raw) -notmatch '(?m)^Pkg\.Revision\s*=\s*28\.2\.13676358\s*$') {
    throw 'NDK revision differs from the pinned 28.2.13676358 toolchain.'
}
New-Item -ItemType Directory -Path $sourceRoot, $build, $logs, $tmp -Force | Out-Null
$lock = [IO.File]::Open((Join-Path $work 'llama-build.lock'), [IO.FileMode]::OpenOrCreate, [IO.FileAccess]::ReadWrite, [IO.FileShare]::None)
$previous = @{}
$overrides = @{ TEMP = $tmp; TMP = $tmp; CC = $null; CXX = $null; CFLAGS = $null; CXXFLAGS = $null; LDFLAGS = $null }
try {
    foreach ($name in $overrides.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        if ($null -eq $overrides[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $overrides[$name], 'Process') }
    }
    if (-not (Test-Path -LiteralPath $archive)) {
        $download = Join-Path $tmp ('llama-' + [Guid]::NewGuid().ToString('N') + '.zip')
        try {
            Invoke-WebRequest -Uri $url -OutFile $download -TimeoutSec 180 -UseBasicParsing
            if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $archiveSha256) { throw 'Official archive SHA256 differs from the source pin.' }
            Assert-IsolatedPath $download $tmp
            Assert-IsolatedPath $archive $sourceRoot
            Move-Item -LiteralPath $download -Destination $archive
        } finally { if (Test-Path -LiteralPath $download) { Remove-Item -LiteralPath $download -Force } }
    }
    if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash -ne $archiveSha256) { throw 'Cached archive SHA256 differs from the source pin; cache was not reused.' }
    if (-not (Test-Path -LiteralPath $source)) {
        $extract = Join-Path $tmp ('llama-extract-' + [Guid]::NewGuid().ToString('N'))
        try {
            Expand-Archive -LiteralPath $archive -DestinationPath $extract
            $extracted = Join-Path $extract "llama.cpp-$commit"
            Assert-IsolatedPath $extracted $tmp
            Assert-IsolatedPath $source $sourceRoot
            Move-Item -LiteralPath $extracted -Destination $source
        } finally {
            if (Test-Path -LiteralPath $extract) { Assert-IsolatedPath $extract $tmp; Remove-Item -LiteralPath $extract -Recurse -Force }
        }
    }
    $sourceVerification = Test-PinnedSource
    $oldManifest = $null
    $cachedArtifactVerified = $false
    if (Test-Path -LiteralPath $manifestPath) {
        $oldManifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
        if (Test-Path -LiteralPath $binary) {
            if ((Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash -ne $oldManifest.artifact.sha256) { throw 'Cached executable hash changed; refusing to reuse an unverified output.' }
            $cachedArtifactVerified = $true
        }
    }
    $options = [ordered]@{
        CMAKE_MAKE_PROGRAM = $ninja
        CMAKE_TOOLCHAIN_FILE = (Join-Path $ndk 'build\cmake\android.toolchain.cmake')
        CMAKE_BUILD_TYPE = 'Release'; ANDROID_ABI = 'arm64-v8a'; ANDROID_PLATFORM = 'android-24'
        ANDROID_STL = 'c++_static'; ANDROID_SUPPORT_FLEXIBLE_PAGE_SIZES = 'ON'
        CMAKE_POSITION_INDEPENDENT_CODE = 'ON'; CMAKE_C_FLAGS = '-march=armv8-a'; CMAKE_CXX_FLAGS = '-march=armv8-a'
        # NDK Release includes debug information; keep the deployable executable stripped.
        CMAKE_EXE_LINKER_FLAGS = '-Wl,-z,max-page-size=16384 -Wl,-z,common-page-size=16384 -Wl,--strip-all -pie'
        BUILD_SHARED_LIBS = 'OFF'; GGML_BACKEND_DL = 'OFF'; GGML_CPU = 'ON'; GGML_CPU_ARM_ARCH = 'armv8-a'
        GGML_NATIVE = 'OFF'; GGML_CPU_ALL_VARIANTS = 'OFF'; GGML_OPENMP = 'OFF'; GGML_OPENMP_FETCH = 'OFF'
        GGML_CUDA = 'OFF'; GGML_VULKAN = 'OFF'; GGML_METAL = 'OFF'; GGML_BLAS = 'OFF'
        GGML_OPENCL = 'OFF'; GGML_SYCL = 'OFF'; GGML_RPC = 'OFF'; GGML_CPU_KLEIDIAI = 'OFF'; GGML_CPU_HBM = 'OFF'
        LLAMA_BUILD_COMMON = 'ON'; LLAMA_BUILD_TOOLS = 'ON'; LLAMA_BUILD_SERVER = 'ON'
        LLAMA_BUILD_TESTS = 'OFF'; LLAMA_BUILD_EXAMPLES = 'OFF'; LLAMA_BUILD_APP = 'OFF'
        LLAMA_BUILD_UI = 'OFF'; LLAMA_USE_PREBUILT_UI = 'OFF'; LLAMA_OPENSSL = 'OFF'
        LLAMA_SUBPROCESS = 'OFF'; LLAMA_LLGUIDANCE = 'OFF'; LLAMA_TOOLS_INSTALL = 'OFF'
        LLAMA_BUILD_NUMBER = '0'; LLAMA_BUILD_COMMIT = $commit
        # ZIP source has no Git metadata. Prevent upstream metadata probes from invoking Git.
        CMAKE_DISABLE_FIND_PACKAGE_Git = 'ON'; GIT_EXECUTABLE = ''
    }
    $configureArgs = @('-S', $source, '-B', $build, '-G', 'Ninja')
    foreach ($name in $options.Keys) { $configureArgs += "-D$name=$($options[$name])" }
    $configureResult = Invoke-LlamaTool $cmake $configureArgs 'llama-configure' 180
    $buildResult = Invoke-LlamaTool $cmake @('--build', $build, '--target', 'llama-server', '--parallel', [string]$Jobs) 'llama-build' $BuildTimeoutSeconds
    if (-not (Test-Path -LiteralPath $binary -PathType Leaf)) { throw 'llama-server executable was not produced.' }
    $compileCommands = Get-Content -LiteralPath (Join-Path $build 'compile_commands.json') -Raw | ConvertFrom-Json
    foreach ($command in $compileCommands.command) {
        $arches = [regex]::Matches($command, '-march=([^\s"]+)')
        if ($arches.Count -eq 0 -or @($arches | Where-Object { $_.Groups[1].Value -ne 'armv8-a' }).Count -gt 0 -or
            $command -match '-mcpu=|-DGGML_USE_(MATMUL_INT8|DOTPROD|SVE|SME)') { throw 'Compile command enabled a CPU feature outside generic ARMv8-A.' }
    }
    $elfResult = Invoke-LlamaTool $readelf @('-h', '-lW', '-d', $binary) 'llama-elf' 30
    $elf = Get-Content -LiteralPath $elfResult.stdout
    $elfText = $elf -join "`n"
    if ($elfText -notmatch 'Machine:\s+AArch64' -or $elfText -notmatch 'Type:\s+DYN' -or
        $elfText -notmatch '\(FLAGS_1\).*\bPIE\b' -or $elfText -notmatch 'Requesting program interpreter:\s*/system/bin/linker64\]') {
        throw 'Output is not an Android AArch64 PIE executable using /system/bin/linker64.'
    }
    $loads = @($elf | Where-Object { $_ -match '^\s*LOAD\s' })
    if ($loads.Count -eq 0) { throw 'No ELF LOAD segments.' }
    $alignments = foreach ($segment in $loads) {
        if ($segment -notmatch '(0x[0-9a-f]+)\s*$') { throw "Cannot read ELF LOAD alignment: $segment" }
        $alignment = [Convert]::ToInt64($Matches[1].Substring(2), 16)
        if ($alignment -lt 16384) { throw "ELF LOAD alignment below 16 KiB: $segment" }
        $alignment
    }
    $needed = @([regex]::Matches($elfText, '\(NEEDED\).*\[([^\]]+)\]') | ForEach-Object { $_.Groups[1].Value })
    if ($needed.Count -eq 0) { throw 'No Android system dependencies found.' }
    foreach ($name in $needed) {
        if ($name -notin @('libm.so', 'libc.so', 'libdl.so', 'liblog.so')) { throw "Unpackaged non-system dependency: $name" }
    }
    $artifactHash = (Get-FileHash -LiteralPath $binary -Algorithm SHA256).Hash
    New-Item -ItemType Directory -Path (Split-Path -Parent $packaged) -Force | Out-Null
    Copy-Item -LiteralPath $binary -Destination $packaged -Force
    if ((Get-FileHash -LiteralPath $packaged -Algorithm SHA256).Hash -ne $artifactHash) { throw 'Packaged executable hash differs from the build output.' }
    $manifest = [ordered]@{
        schemaVersion = 1; stage = '2/3'; completedAt = [DateTimeOffset]::Now.ToString('o')
        source = [ordered]@{ repository = 'ggml-org/llama.cpp'; commit = $commit; url = $url; archive = $archive; archiveSha256 = $archiveSha256; archiveBytes = (Get-Item -LiteralPath $archive).Length; path = $source; verification = $sourceVerification }
        toolchain = [ordered]@{ ndkVersion = '28.2.13676358'; cmakeVersion = '3.22.1'; api = 24; abi = 'arm64-v8a'; cpu = 'armv8-a'; stl = 'c++_static'; tools = @(@($cmake, $ninja, (Join-Path $ndkBin 'clang.exe'), $readelf) | ForEach-Object { [ordered]@{ path = $_; sha256 = (Get-FileHash -LiteralPath $_ -Algorithm SHA256).Hash } }) }
        build = [ordered]@{ options = $options; target = 'llama-server'; jobs = $Jobs; cachedArtifactVerified = $cachedArtifactVerified; configure = $configureResult; compile = $buildResult; readelf = $elfResult; compileCommandsGenericArmv8Verified = $true }
        artifact = [ordered]@{ path = $binary; packagedPath = $packaged; bytes = (Get-Item -LiteralPath $binary).Length; sha256 = $artifactHash; packagedSha256 = $artifactHash; elfType = 'DYN'; machine = 'AArch64'; pie = $true; interpreter = '/system/bin/linker64'; loadAlignments = @($alignments); needed = $needed }
        deviceValidation = $null; apkAssembled = $false; apkInstalled = $false; modelsDownloaded = $false
    }
    if ($oldManifest -and $oldManifest.artifact.sha256 -eq $artifactHash) { $manifest.deviceValidation = $oldManifest.deviceValidation }
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 12), $utf8)
    if ($Serial) {
        $adb = Join-Path $sdk 'platform-tools\adb.exe'
        if (-not (Test-Path -LiteralPath $adb)) { throw "Installed adb missing: $adb" }
        $remote = '/data/local/tmp/modularchat-llama-step2-' + $commit.Substring(0, 8) + '-' + [Guid]::NewGuid().ToString('N')
        $remoteBin = "$remote/libllama_server.so"
        $device = [ordered]@{ serial = $Serial; remoteDirectory = $remote; artifactSha256 = $artifactHash; completed = $false; cleanupCompleted = $false }
        $manifest.deviceValidation = $device
        try {
            $device.state = Invoke-LlamaTool $adb @('-s', $Serial, 'get-state') 'llama-adb-state' 20
            $device.properties = Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'getprop', 'ro.product.cpu.abi') 'llama-adb-abi' 20
            if ((Get-Content -LiteralPath $device.properties.stdout -Raw).Trim() -ne 'arm64-v8a') { throw 'Selected device is not ARM64.' }
            [void](Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'mkdir', $remote) 'llama-adb-mkdir' 20)
            $device.push = Invoke-LlamaTool $adb @('-s', $Serial, 'push', $packaged, $remoteBin) 'llama-adb-push' 60
            [void](Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'chmod', '700', $remoteBin) 'llama-adb-chmod' 20)
            $device.hash = Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'toybox', 'sha256sum', $remoteBin) 'llama-adb-hash' 30
            if ((Get-Content -LiteralPath $device.hash.stdout -Raw).Split(' ')[0].Trim() -ne $artifactHash) { throw 'Device executable SHA256 differs from packaged output.' }
            $device.systemDependencies = @()
            foreach ($name in $needed) {
                $dependency = Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'test', '-r', "/system/lib64/$name") ('llama-adb-dependency-' + $name) 20
                $device.systemDependencies += [ordered]@{ name = $name; path = "/system/lib64/$name"; exitCode = $dependency.exitCode }
            }
            $device.version = Invoke-LlamaTool $adb @('-s', $Serial, 'shell', $remoteBin, '--version') 'llama-adb-version' 30
            $device.versionText = ((Get-Content -LiteralPath $device.version.stdout -Raw) + (Get-Content -LiteralPath $device.version.stderr -Raw)).Trim()
            if ($device.versionText -notmatch [regex]::Escape($commit)) { throw 'Device --version does not report the pinned source commit.' }
            $device.help = Invoke-LlamaTool $adb @('-s', $Serial, 'shell', $remoteBin, '--help') 'llama-adb-help' 30
            $help = (Get-Content -LiteralPath $device.help.stdout -Raw) + (Get-Content -LiteralPath $device.help.stderr -Raw)
            $device.requiredFlags = @('--no-webui', '--sleep-idle-seconds', '--device', '-ngl', '-t', '-tb', '-c', '-np', '-b', '-ub', '--cache-ram', '--jinja', '--reasoning', '--embeddings', '--pooling')
            foreach ($flag in $device.requiredFlags) {
                if ($help -notmatch ('(?m)(^|[ ,])' + [regex]::Escape($flag) + '(?=[ ,\r\n])')) { throw "Required mobile flag absent from help: $flag" }
            }
            $baseArgs = @('--no-webui', '--sleep-idle-seconds', '-1', '--device', 'none', '-ngl', '0', '-t', '4', '-tb', '4', '-np', '1')
            $device.chatProfile = Invoke-LlamaTool $adb (@('-s', $Serial, 'shell', $remoteBin) + $baseArgs + @('-c', '4096', '-b', '128', '-ub', '64', '--cache-ram', '0', '--jinja', '--reasoning', 'off', '--help')) 'llama-adb-chat-profile' 30
            $device.embeddingProfile = Invoke-LlamaTool $adb (@('-s', $Serial, 'shell', $remoteBin) + $baseArgs + @('--embeddings', '--pooling', 'mean', '-c', '2048', '-b', '2048', '-ub', '2048', '--help')) 'llama-adb-embedding-profile' 30
            $device.completed = $true
            $device.completedAt = [DateTimeOffset]::Now.ToString('o')
            Write-Host $device.versionText
        } finally {
            try {
                if ($remote -notmatch '^/data/local/tmp/modularchat-llama-step2-[0-9a-f]{8}-[0-9a-f]{32}$') { throw 'Unsafe device cleanup path.' }
                [void](Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'rm', '-f', $remoteBin) 'llama-adb-cleanup-bin' 20)
                [void](Invoke-LlamaTool $adb @('-s', $Serial, 'shell', 'rmdir', $remote) 'llama-adb-cleanup-dir' 20)
                $device.cleanupCompleted = $true
            } catch { Write-Warning "Task device cleanup failed: $_" }
            [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 12), $utf8)
        }
    }
    Write-Host "ARM64 PIE verified: $packaged ($($manifest.artifact.bytes) bytes; SHA256 $artifactHash)"
    Write-Host "Manifest: $manifestPath"
} finally {
    foreach ($name in $previous.Keys) {
        if ($null -eq $previous[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    }
    $lock.Dispose()
}

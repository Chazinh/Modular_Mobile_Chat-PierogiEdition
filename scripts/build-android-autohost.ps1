# Build only the Android ARM64 Rust/JNI host; never sync frontend, assemble or install an APK.
[CmdletBinding()]
param(
    [string]$Local = 'C:\Users\Administrator\RoleplayChat-local',
    [switch]$ValidateJava
)
$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$work = Join-Path $Local 'build-isolated\android-autohost'
$sdk = Join-Path $Local 'android-sdk'
$ndk = Join-Path $sdk 'ndk\28.2.13676358'
$bin = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$clang = Join-Path $bin 'aarch64-linux-android24-clang.cmd'
$readelf = Join-Path $bin 'llvm-readelf.exe'
function Invoke-BuildProcess([string]$Executable, [string]$Arguments, [string]$Directory, [string]$Log) {
    $info = New-Object Diagnostics.ProcessStartInfo
    $info.FileName = $Executable
    $info.Arguments = $Arguments
    $info.WorkingDirectory = $Directory
    $info.UseShellExecute = $false
    $info.CreateNoWindow = $true
    $info.RedirectStandardOutput = $true
    $info.RedirectStandardError = $true
    $process = New-Object Diagnostics.Process
    $process.StartInfo = $info
    if (-not $process.Start()) { throw 'Build process did not start.' }
    $stdout = $process.StandardOutput.ReadToEndAsync()
    $stderr = $process.StandardError.ReadToEndAsync()
    if (-not $process.WaitForExit(600000)) {
        throw "Build exceeded 10 minutes; process $($process.Id) is still running. Do not start a duplicate build."
    }
    $exitCode = $process.ExitCode
    [IO.File]::WriteAllText($Log, ($stdout.GetAwaiter().GetResult() + $stderr.GetAwaiter().GetResult()), (New-Object Text.UTF8Encoding $false))
    $process.Dispose()
    Write-Output "Build process exit=$exitCode; log=$Log"
    if ($exitCode -ne 0) {
        Get-Content -LiteralPath $Log -Tail 24
        throw "Build failed (exit $exitCode). Full log: $Log"
    }
}
foreach ($required in @($clang, $readelf, (Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\include\jni.h'), (Join-Path $sdk 'cmake\3.22.1\bin\cmake.exe'))) {
    if (-not (Test-Path -LiteralPath $required)) { throw "Official toolchain missing: $required. Install ndk;28.2.13676358 and cmake;3.22.1 with sdkmanager." }
}
$installedTargets = & rustup target list --installed
if ($LASTEXITCODE -ne 0 -or $installedTargets -notcontains 'aarch64-linux-android') { throw 'Run rustup target add aarch64-linux-android first.' }
New-Item -ItemType Directory -Path $work, (Join-Path $work 'tmp') -Force | Out-Null
$overrides = @{
    CC_aarch64_linux_android = $clang
    AR_aarch64_linux_android = (Join-Path $bin 'llvm-ar.exe')
    CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER = $clang
    CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS = '-C link-arg=-Wl,-z,max-page-size=16384 -C link-arg=-Wl,-z,common-page-size=16384 -C link-arg=-Wl,-soname,libmodularchat_server.so'
    CARGO_TARGET_DIR = (Join-Path $work 'cargo-target')
    RUSTFLAGS = $null
    CARGO_ENCODED_RUSTFLAGS = $null
    TEMP = (Join-Path $work 'tmp')
    TMP = (Join-Path $work 'tmp')
}
if ($ValidateJava) {
    $jdk = Get-ChildItem -LiteralPath (Join-Path $Local 'jdk') -Directory | Where-Object { Test-Path (Join-Path $_.FullName 'bin\java.exe') } | Select-Object -First 1
    if (-not $jdk) { throw 'JDK 21 local not found.' }
    $overrides.JAVA_HOME = $jdk.FullName
    $overrides.ANDROID_HOME = $sdk
    $overrides.GRADLE_USER_HOME = Join-Path $Local 'gradle-home'
    $overrides.MC_ANDROID_NATIVE_BUILD_DIR = Join-Path $work 'cmake'
    $signing = Join-Path $Local 'signing\keystore.properties'
    if (Test-Path -LiteralPath $signing) { $overrides.MC_KEYSTORE_PROPS = $signing }
}
$previous = @{}
try {
    foreach ($name in $overrides.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        # PowerShell can coerce a null value to an empty environment variable;
        # empty RUSTFLAGS/encoded flags override Cargo's target-specific flags.
        if ($null -eq $overrides[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $overrides[$name], 'Process') }
    }
    $rustLog = Join-Path $work 'rust-arm64-release.log'
    $cargoArgs = 'build --manifest-path "' + (Join-Path $project 'server-rust\Cargo.toml') + '" --locked --target aarch64-linux-android --release --lib'
    Invoke-BuildProcess (Get-Command cargo -CommandType Application).Source $cargoArgs $project $rustLog
    $library = Join-Path $overrides.CARGO_TARGET_DIR 'aarch64-linux-android\release\libmodularchat_server.so'
    $elf = & $readelf -h -lW -SW -d --dyn-syms $library
    if ($LASTEXITCODE -ne 0) { throw 'llvm-readelf failed.' }
    $elfLog = Join-Path $work 'rust-arm64-elf.txt'
    [IO.File]::WriteAllLines($elfLog, [string[]]$elf)
    $text = $elf -join "`n"
    if ($text -notmatch 'Machine:\s+AArch64' -or $text -notmatch 'Type:\s+DYN') { throw 'Runtime is not an AArch64 shared ELF.' }
    $loads = @($elf | Where-Object { $_ -match '^\s*LOAD\s' })
    if ($loads.Count -eq 0) { throw 'No ELF LOAD segments.' }
    foreach ($segment in $loads) {
        if ($segment -notmatch '(0x[0-9a-f]+)\s*$' -or [Convert]::ToInt64($Matches[1].Substring(2), 16) -lt 16384) { throw "ELF LOAD alignment is below 16 KiB: $segment" }
    }
    foreach ($symbol in @('mc_android_start', 'mc_android_stop', 'mc_android_status', 'mc_android_last_error')) {
        if ($text -notmatch "\b$symbol\b") { throw "Missing C ABI export: $symbol" }
    }
    if ($text -notmatch '\(SONAME\).*\[libmodularchat_server\.so\]') { throw 'Runtime SONAME missing.' }
    foreach ($line in ($elf | Where-Object { $_ -match '\(NEEDED\)' })) {
        if ($line -match '\[(.+)\]' -and $Matches[1] -notin @('libc.so', 'libdl.so', 'libm.so', 'liblog.so')) { throw "Unpackaged non-system native dependency: $($Matches[1])" }
    }
    $destination = Join-Path $project 'client\android\app\src\main\jniLibs\arm64-v8a'
    New-Item -ItemType Directory -Path $destination -Force | Out-Null
    Copy-Item -LiteralPath $library -Destination (Join-Path $destination 'libmodularchat_server.so') -Force
    Write-Output "Rust ARM64 release OK; ABI v1 exports and LOAD alignment >=16 KiB verified. Runtime: $destination\libmodularchat_server.so"
    if ($ValidateJava) {
        $gradleInit = Join-Path $work 'isolated.gradle'
        $gradleBuild = (Join-Path $work 'gradle').Replace('\', '/')
        [IO.File]::WriteAllText($gradleInit, "allprojects { layout.buildDirectory.set(new File('$gradleBuild', project.path.replace(':', '_'))) }", (New-Object Text.UTF8Encoding $false))
        $gradleLog = Join-Path $work 'gradle-native-java.log'
        Push-Location (Join-Path $project 'client\android')
        try {
            $gradleArgs = '/d /s /c ""' + (Join-Path $project 'client\android\gradlew.bat') + '" --no-daemon --console=plain --init-script "' + $gradleInit + '" :app:compileReleaseJavaWithJavac :app:externalNativeBuildRelease :app:mergeReleaseNativeLibs"'
            Invoke-BuildProcess $env:ComSpec $gradleArgs (Get-Location).Path $gradleLog
        } finally { Pop-Location }
        $bridge = Get-ChildItem -LiteralPath (Join-Path $work 'gradle\_app\intermediates\cxx') -Filter libmc_android_bridge.so -Recurse -File | Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
        if (-not $bridge) { throw 'JNI output was not produced.' }
        $bridgeElf = & $readelf -h -lW -d --dyn-syms $bridge.FullName
        if ($LASTEXITCODE -ne 0) { throw 'JNI llvm-readelf failed.' }
        [IO.File]::WriteAllLines((Join-Path $work 'jni-arm64-elf.txt'), [string[]]$bridgeElf)
        $bridgeText = $bridgeElf -join "`n"
        if ($bridgeText -notmatch 'Machine:\s+AArch64' -or $bridgeText -notmatch 'Type:\s+DYN' -or
            $bridgeText -notmatch '\(SONAME\).*\[libmc_android_bridge\.so\]' -or
            $bridgeText -notmatch '\(NEEDED\).*\[libmodularchat_server\.so\]') { throw 'JNI ELF architecture, SONAME or Rust dependency invalid.' }
        foreach ($segment in ($bridgeElf | Where-Object { $_ -match '^\s*LOAD\s' })) {
            if ($segment -notmatch '(0x[0-9a-f]+)\s*$' -or [Convert]::ToInt64($Matches[1].Substring(2), 16) -lt 16384) { throw 'JNI ELF LOAD alignment is below 16 KiB.' }
        }
        foreach ($symbol in @('nativeStart', 'nativeStop', 'nativeStatus', 'nativeLastError')) {
            if ($bridgeText -notmatch "\bJava_io_github_modularchat_pierogiedition_NativeHost_$symbol\b") { throw "Missing JNI export: $symbol" }
        }
        # CMake owns packaging of the bridge; copying it to jniLibs would duplicate AGP's generated library.
        $merged = Join-Path $work 'gradle\_app\intermediates\merged_native_libs\release\mergeReleaseNativeLibs\out\lib\arm64-v8a'
        foreach ($name in @('libmodularchat_server.so', 'libmc_android_bridge.so')) {
            if (-not (Test-Path -LiteralPath (Join-Path $merged $name))) { throw "Missing merged native library: $name" }
        }
        Write-Output "JNI output: $($bridge.FullName); merged ARM64 libraries: $merged"
        Write-Output "Release Java, JNI/CMake and native-library merge OK. Log: $gradleLog"
    }
} finally {
    foreach ($name in $previous.Keys) {
        if ($null -eq $previous[$name]) { Remove-Item -LiteralPath "Env:$name" -ErrorAction SilentlyContinue }
        else { [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process') }
    }
}

$ErrorActionPreference = 'Stop'
$project = Split-Path -Parent $PSScriptRoot
$ast = [Management.Automation.Language.Parser]::ParseFile((Join-Path $project 'scripts\build-release-android.ps1'), [ref]$null, [ref]$null)
$fn = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Invoke-ReleaseTool' }, $true)
if (-not $fn) { throw 'Release launcher function missing.' }
Invoke-Expression $fn.Extent.Text
$root = Join-Path ([IO.Path]::GetTempPath()) ('mc-release-tool-' + [Guid]::NewGuid().ToString('N'))
$logs = Join-Path $root 'logs'
$steps = New-Object Collections.Generic.List[object]
try {
    New-Item -ItemType Directory -Path $logs -Force | Out-Null
    $batch = Join-Path $root 'tool with spaces.cmd'
    [IO.File]::WriteAllText($batch, "@echo off`r`necho argument=[%~1]`r`necho diagnostic 1>&2`r`nexit /b 0`r`n")
    $result = Invoke-ReleaseTool $batch @('value with spaces') 'batch'
    if ($result -notmatch 'argument=\[value with spaces\]') { throw 'Batch quoting lost an argument.' }
    if ((Get-Content (Join-Path $logs 'batch.stderr.log') -Raw) -notmatch 'diagnostic') { throw 'Missing stderr log.' }
    [IO.File]::WriteAllText($batch, "@echo off`r`nexit /b 7`r`n")
    $failed = $false
    try { Invoke-ReleaseTool $batch @() 'failure' | Out-Null } catch {
        if ($_.Exception.Message -notmatch 'exit 7') { throw }
        $failed = $true
    }
    if (-not $failed) { throw 'Nonzero batch exit was swallowed.' }
    $source = [IO.File]::ReadAllText((Join-Path $project 'scripts\build-release-android.ps1'))
    $calls = [regex]::Matches($source, '(?m)^\s*\[void\]\(Invoke-ReleaseTool (\(Get-Command (?:npm|npx)\.cmd[^\r\n]+?\)\.Source)')
    if ($calls.Count -ne 2) { throw 'Expected npm and npx release calls.' }
    $oldPath = $env:Path
    try {
        $first = Join-Path $root 'first'; $second = Join-Path $root 'second'
        New-Item -ItemType Directory -Path $first,$second | Out-Null
        foreach ($name in @('npm.cmd','npx.cmd')) {
            [IO.File]::WriteAllText((Join-Path $first $name), "@echo off`r`necho first`r`n")
            [IO.File]::WriteAllText((Join-Path $second $name), "@echo off`r`necho second`r`n")
        }
        $env:Path = "$first;$second;$oldPath"
        foreach ($call in $calls) {
            $resolved = @(Invoke-Expression $call.Groups[1].Value)
            if ($resolved.Count -ne 1 -or -not $resolved[0].StartsWith($first)) { throw 'Multiple PATH matches were combined into one executable path.' }
            $result = Invoke-ReleaseTool $resolved[0] @('--version') ('path-' + [IO.Path]::GetFileNameWithoutExtension($resolved[0]))
            if ($result -notmatch 'first') { throw 'Wrong PATH candidate executed.' }
        }
    } finally { $env:Path = $oldPath }
    Write-Output 'PASS: spaced batch path, spaced arguments, stderr, nonzero exit and duplicate npm/npx PATH entries.'
} finally {
    if (Test-Path -LiteralPath $root) { Remove-Item -LiteralPath $root -Recurse -Force }
}

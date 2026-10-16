<#
.SYNOPSIS
    Automação para rodar todas as suítes de teste do ModularChat.
.DESCRIPTION
    Executa o runner automatizado em Node.js com suporte a filtros, execução seletiva de testes unitários,
    integração, Rust (cargo test) e validação de build.
.EXAMPLE
    .\scripts\run-all-tests.ps1
    .\scripts\run-all-tests.ps1 --unit
    .\scripts\run-all-tests.ps1 --rust
    .\scripts\run-all-tests.ps1 --filter=server-address
#>
[CmdletBinding()]
param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$RunnerArgs
)

$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RunnerScript = Join-Path $ScriptDir "run-all-tests.mjs"

Write-Host "Invocando runner de testes automatizado..." -ForegroundColor Cyan
& node $RunnerScript @RunnerArgs

exit $LASTEXITCODE

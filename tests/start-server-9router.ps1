# Teste isolado para suporte ao 9Router no inicializador do servidor (start-server.ps1 e start-server.bat)
$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$scriptPath = Join-Path $root 'start-server.ps1'
$batPath = Join-Path $root 'start-server.bat'

# 1. Validacao sintatica do script start-server.ps1
$tokens = $null
$parseErrors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($scriptPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -gt 0) {
    throw "Erro de sintaxe em start-server.ps1: $($parseErrors[0].Message)"
}

# 2. Verificar que as funcoes Find-NineRouterExecutable e Start-NineRouter estao presentes na AST
$findFuncAst = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Find-NineRouterExecutable' }, $true)
if (-not $findFuncAst) { throw 'Funcao Find-NineRouterExecutable nao encontrada na AST de start-server.ps1.' }

$startFuncAst = $ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Start-NineRouter' }, $true)
if (-not $startFuncAst) { throw 'Funcao Start-NineRouter nao encontrada na AST de start-server.ps1.' }

# Carregar funcoes em escopo isolado
Invoke-Expression $findFuncAst.Extent.Text
Invoke-Expression $startFuncAst.Extent.Text

# 3. Teste de Find-NineRouterExecutable com mocks
$mockState = [pscustomobject]@{
    MockCommand = $null
    MockPathExists = @{}
    StartedProcesses = [Collections.Generic.List[object]]::new()
    ListeningPorts = @{}
}

function Get-Command {
    param($Name, $ErrorAction)
    if ($mockState.MockCommand -and $Name -eq '9router.cmd') {
        return [pscustomobject]@{ Source = $mockState.MockCommand }
    }
    return $null
}

function Test-Path {
    param($LiteralPath)
    if ($mockState.MockPathExists.ContainsKey($LiteralPath)) {
        return $mockState.MockPathExists[$LiteralPath]
    }
    return $false
}

# Cenário 3a: Encontra 9router.cmd via Get-Command
$mockState.MockCommand = 'C:\fake\npm\9router.cmd'
$exe = Find-NineRouterExecutable
if ($exe -ne 'C:\fake\npm\9router.cmd') { throw "Find-NineRouterExecutable falhou ao resolver via Get-Command: $exe" }

# Cenário 3b: Fallback para arquivo em %APPDATA%\npm\9router.cmd
$mockState.MockCommand = $null
$fallbackCmd = Join-Path $env:APPDATA 'npm\9router.cmd'
$mockState.MockPathExists[$fallbackCmd] = $true
$exe = Find-NineRouterExecutable
if ($exe -ne $fallbackCmd) { throw "Find-NineRouterExecutable falhou no fallback para %APPDATA%\npm\9router.cmd: $exe" }
$mockState.MockPathExists.Remove($fallbackCmd)

# Cenário 3c: Fallback para cli.js
$cliJs = Join-Path $env:APPDATA 'npm\node_modules\9router\cli.js'
$mockState.MockPathExists[$cliJs] = $true
$exe = Find-NineRouterExecutable
if ($exe -ne $cliJs) { throw "Find-NineRouterExecutable falhou no fallback para cli.js: $exe" }
$mockState.MockPathExists.Remove($cliJs)

# Cenário 3d: Nenhum executavel disponivel
$exe = Find-NineRouterExecutable
if ($null -ne $exe) { throw 'Find-NineRouterExecutable deveria retornar $null quando nao encontrado.' }

# 4. Teste de Start-NineRouter
function Get-NetTCPConnection {
    param($State, $LocalPort, $ErrorAction)
    if ($mockState.ListeningPorts.ContainsKey($LocalPort) -and $mockState.ListeningPorts[$LocalPort]) {
        return [pscustomobject]@{ LocalPort = $LocalPort; OwningProcess = 9999 }
    }
    return $null
}

function Start-Process {
    param($FilePath, $ArgumentList, $WindowStyle, [switch]$PassThru)
    $mockState.StartedProcesses.Add([pscustomobject]@{
        FilePath = $FilePath
        ArgumentList = $ArgumentList
        WindowStyle = $WindowStyle
    })
    # Simula que a porta fica disponivel apos o spawn
    $mockState.ListeningPorts[20128] = $true
    return [pscustomobject]@{ Id = 12345 }
}

# Cenário 4a: Porta já está em execução -> não inicia novo processo
$mockState.ListeningPorts[20128] = $true
$mockState.StartedProcesses.Clear()
$pidResult = Start-NineRouter -TargetPort 20128 -TimeoutSeconds 2
if ($mockState.StartedProcesses.Count -ne 0) { throw 'Start-NineRouter nao deveria iniciar processo quando a porta ja esta ativa.' }
if ($pidResult -ne 9999) { throw "Start-NineRouter deveria retornar o PID existente 9999, retornou: $pidResult" }

# Cenário 4b: Porta nao esta ativa -> inicia com os argumentos corretos
$mockState.ListeningPorts.Remove(20128)
$mockState.MockCommand = 'C:\fake\npm\9router.cmd'
$mockState.StartedProcesses.Clear()
$pidResult = Start-NineRouter -TargetPort 20128 -TimeoutSeconds 2
if ($mockState.StartedProcesses.Count -ne 1) { throw 'Start-NineRouter deveria iniciar 1 processo quando a porta esta inativa.' }
$call = $mockState.StartedProcesses[0]
if ($call.FilePath -ne 'cmd.exe') { throw "Invocacao de .cmd deveria usar cmd.exe como launcher seguro: $($call.FilePath)" }
if (-not ($call.ArgumentList -contains '--port' -and $call.ArgumentList -contains '20128' -and $call.ArgumentList -contains '--tray')) {
    throw "Argumentos de inicializacao do 9Router incorretos: $($call.ArgumentList -join ' ')"
}

# Cenário 4c: Executavel nao encontrado -> lanca excecao apropriada
$mockState.ListeningPorts.Remove(20128)
$mockState.MockCommand = $null
$mockState.StartedProcesses.Clear()
$rejected = $false
try {
    Start-NineRouter -TargetPort 20128 -TimeoutSeconds 1
} catch {
    $rejected = $true
}
if (-not $rejected) { throw 'Start-NineRouter deveria falhar quando nenhum executavel e encontrado.' }

# Cenário 4d: Regressão - DATA_DIR isolado para que o 9Router use seu AppData canônico em vez do DATA_DIR do servidor
$mockState.ListeningPorts.Remove(20128)
$mockState.MockCommand = 'C:\fake\npm\9router.cmd'
$mockState.StartedProcesses.Clear()
$env:DATA_DIR = 'C:\fake\modularchat-data'
$capturedDataDirDuringSpawn = $null

function Start-Process {
    param($FilePath, $ArgumentList, $WindowStyle, [switch]$PassThru)
    # Captura o valor de DATA_DIR no exato momento em que o processo filho seria iniciado
    $capturedDataDirDuringSpawn = $env:DATA_DIR
    $mockState.StartedProcesses.Add([pscustomobject]@{
        FilePath = $FilePath
        ArgumentList = $ArgumentList
        WindowStyle = $WindowStyle
    })
    $mockState.ListeningPorts[20128] = $true
    return [pscustomobject]@{ Id = 12345 }
}

$pidResult = Start-NineRouter -TargetPort 20128 -TimeoutSeconds 2
if ($null -ne $capturedDataDirDuringSpawn) {
    throw "Regressao detectada: DATA_DIR nao foi isolado durante o spawn do 9Router! Valor capturado: $capturedDataDirDuringSpawn"
}
if ($env:DATA_DIR -ne 'C:\fake\modularchat-data') {
    throw "DATA_DIR nao foi restaurado no ambiente pai apos a execucao de Start-NineRouter. Valor atual: $($env:DATA_DIR)"
}
Remove-Item Env:DATA_DIR -ErrorAction SilentlyContinue

# 5. Validacao do start-server.bat
$batContent = Get-Content -LiteralPath $batPath -Raw
if ($batContent -notmatch '-With9Router') { throw 'start-server.bat deve conter referencia para -With9Router.' }
if ($batContent -notmatch 'choice' -and $batContent -notmatch '9router') { throw 'start-server.bat deve fornecer opcao/atalho para o 9Router.' }

Write-Output '9Router start server tests passed: AST syntax, Find-NineRouterExecutable, Start-NineRouter idempotency and argument validation confirmed.'

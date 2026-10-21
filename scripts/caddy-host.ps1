# Host Caddy nativo do Windows. Nao inicia, encerra ou recompila o servidor do app.
[CmdletBinding()]
param(
    [ValidateSet('Prepare','Install','Activate','Lan','Local','Trust','Login','PairCode','Status','Stop')]
    [string]$Action='Status',
    [string]$Domain='',
    [string]$LanAddress='',
    [string]$LanSubnet='',
    [string]$Base='',
    [ValidateRange(1024,65535)][int]$BackendPort=3000,
    [ValidateRange(1024,65535)][int]$HttpsPort=8443,
    [ValidateRange(1024,65535)][int]$AdminPort=2019,
    [ValidatePattern('^[a-zA-Z0-9_-]{1,32}$')][string]$WebUser='modularchat'
)
$ErrorActionPreference='Stop'
if(-not $Base){$Base=Join-Path (Split-Path $PSScriptRoot -Parent) 'hosting\caddy'}
$base=[IO.Path]::GetFullPath($Base)
$exe=Join-Path $base 'caddy.exe'
$config=Join-Path $base 'Caddyfile'
$authFile=Join-Path $base 'auth.json'
$serviceName='ModularChatCaddy'
$template=Join-Path (Split-Path $PSScriptRoot -Parent) 'hosting\Caddyfile.template'
$utf8=New-Object System.Text.UTF8Encoding($false)

function Require-Admin {
    $identity=[Security.Principal.WindowsIdentity]::GetCurrent()
    if(-not (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){
        throw 'Abra o PowerShell como Administrador para alterar o servico Caddy.'
    }
}
function Protect-Directory([string]$path,[bool]$serviceWrite=$false) {
    $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    $serviceRight=if($serviceWrite){'M'}else{'RX'}
    & icacls.exe $path '/inheritance:r' '/grant:r' '*S-1-5-32-544:(OI)(CI)F' '*S-1-5-18:(OI)(CI)F' "*${sid}:(OI)(CI)F" "*S-1-5-19:(OI)(CI)$serviceRight" | Out-Null
    if($LASTEXITCODE -ne 0){throw "Nao foi possivel proteger a pasta $path."}
}
function Protect-Secret([string]$path) {
    $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
    & icacls.exe $path '/inheritance:r' '/grant:r' '*S-1-5-32-544:F' '*S-1-5-18:F' "*${sid}:F" | Out-Null
    if($LASTEXITCODE -ne 0){throw "Nao foi possivel proteger o arquivo $path."}
}
function Random-Token([int]$count=32) {
    $bytes=New-Object byte[] $count
    $rng=[Security.Cryptography.RandomNumberGenerator]::Create()
    try{$rng.GetBytes($bytes)}finally{$rng.Dispose()}
    return [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','-').Replace('/','_')
}
function Validate-HostOptions([bool]$public) {
    if($public -and $Domain -notmatch '^(?=.{1,253}$)[a-z0-9](?:[a-z0-9.-]*[a-z0-9])?\.[a-z]{2,}$'){
        throw 'Informe somente o nome DNS, sem https, porta ou caminho.'
    }
    if(@(@($BackendPort,$HttpsPort,$AdminPort) | Select-Object -Unique).Count -ne 3){throw 'As portas do backend, HTTPS local e administracao precisam ser distintas.'}
    if($LanAddress){
        [Net.IPAddress]$lanIp=$null
        if(-not [Net.IPAddress]::TryParse($LanAddress,[ref]$lanIp) -or $lanIp.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork){throw 'Informe um IPv4 LAN valido.'}
        if($LanSubnet -notmatch '^((?:\d{1,3}\.){3}\d{1,3})/(\d|[12]\d|3[0-2])$'){throw 'Informe a sub-rede LAN em formato CIDR.'}
        $parts=$LanSubnet.Split('/');$prefix=[int]$parts[1]
        [Net.IPAddress]$subnetIp=$null
        if(-not [Net.IPAddress]::TryParse($parts[0],[ref]$subnetIp)){throw 'Sub-rede LAN invalida.'}
        $addressBytes=$lanIp.GetAddressBytes();$subnetBytes=$subnetIp.GetAddressBytes()
        $minimum=if($addressBytes[0] -eq 10){8}elseif($addressBytes[0] -eq 172 -and $addressBytes[1] -ge 16 -and $addressBytes[1] -le 31){12}elseif($addressBytes[0] -eq 192 -and $addressBytes[1] -eq 168){16}else{throw 'Use um IPv4 privado para o acesso LAN.'}
        if($prefix -lt $minimum){throw 'A sub-rede LAN nao pode incluir enderecos publicos.'}
        for($bit=0;$bit -lt $prefix;$bit++){
            $mask=1 -shl (7-($bit%8));$idx=[int][Math]::Floor($bit/8)
            if(($addressBytes[$idx] -band $mask) -ne ($subnetBytes[$idx] -band $mask)){throw 'O IP LAN precisa pertencer a sub-rede informada.'}
        }
    }elseif($LanSubnet){throw 'Informe o endereco LAN junto com a sub-rede.'}
}
function Write-Config([bool]$public) {
    Validate-HostOptions $public
    $auth=Get-Content -LiteralPath $authFile -Raw | ConvertFrom-Json
    $lanEnabled=($public -or $Action -eq 'Lan') -and $LanAddress
    $publicSite=if($public){"https://$Domain {`n`timport modularchat_gateway`n}"}else{''}
    $localSite="https://localhost:$HttpsPort {`n`tbind 127.0.0.1`n`ttls internal`n`timport modularchat_gateway`n}"
    $lanSite=if($lanEnabled){"https://${LanAddress}:$HttpsPort {`n`tbind $LanAddress`n`ttls internal`n`timport modularchat_gateway`n}"}else{''}
    $content=[IO.File]::ReadAllText($template).Replace('__RUNTIME__',$base.Replace('\','/')).Replace('__PASSWORD_HASH__',$auth.hash).Replace('__SESSION_TOKEN__',$auth.session).Replace('__WEB_USER__',$auth.user).Replace('__ADMIN_ADDRESS__',"127.0.0.1:$AdminPort").Replace('__BACKEND_PORT__',"$BackendPort").Replace('__LAN_SUBNET__',$(if($lanEnabled){$LanSubnet}else{''})).Replace('__LOCAL_SITE__',$localSite).Replace('__PUBLIC_SITE__',$publicSite).Replace('__LAN_SITE__',$lanSite)
    $candidate=Join-Path $base 'Caddyfile.next'
    [IO.File]::WriteAllText($candidate,$content,$utf8)
    $validation=Join-Path $base 'logs\config-validation.log'
    # Windows PowerShell 5 trata ate os avisos JSON do stderr nativo como erros.
    $savedPreference=$ErrorActionPreference
    try{
        $ErrorActionPreference='Continue'
        & $exe validate --config $candidate --adapter caddyfile *> $validation
        $validationExit=$LASTEXITCODE
    }finally{$ErrorActionPreference=$savedPreference}
    if($validationExit -ne 0){throw "Configuracao recusada pelo Caddy; o arquivo ativo foi preservado. Diagnostico em $validation"}
    if(Test-Path -LiteralPath $config){Copy-Item -LiteralPath $config -Destination (Join-Path $base 'Caddyfile.previous') -Force}
    Move-Item -LiteralPath $candidate -Destination $config -Force
    $service=Get-Service -Name $serviceName -ErrorAction SilentlyContinue
    if($service -and $service.Status -eq 'Running'){
        try{
            $ErrorActionPreference='Continue'
            & $exe reload --address "127.0.0.1:$AdminPort" --config $config --adapter caddyfile *> (Join-Path $base 'logs\reload.log')
            $reloadExit=$LASTEXITCODE
        }finally{$ErrorActionPreference=$savedPreference}
        if($reloadExit -ne 0){
            $previous=Join-Path $base 'Caddyfile.previous'
            if(Test-Path -LiteralPath $previous){Copy-Item -LiteralPath $previous -Destination $config -Force}
            throw 'Nao foi possivel recarregar; a configuracao anterior foi restaurada no disco e o servico em execucao foi preservado.'
        }
    }
    [IO.File]::WriteAllText((Join-Path $base 'state.json'),(@{domain=$Domain;public=$public;lanAddress=$(if($lanEnabled){$LanAddress}else{''});httpsPort=$HttpsPort;backendPort=$BackendPort;updatedAt=(Get-Date).ToString('o')}|ConvertTo-Json),$utf8)
}

switch($Action){
    {$_ -in @('Prepare','Install')} {
        Validate-HostOptions $false
        if(-not (Test-Path -LiteralPath $exe)){throw "Instale o binario oficial verificado em $exe antes de continuar."}
        if(Get-Service -Name $serviceName -ErrorAction SilentlyContinue){throw 'O servico ja existe. Use Activate, Local ou Status; nenhuma instancia sera substituida.'}
        $occupied=Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object LocalPort -in $HttpsPort,$AdminPort
        if($occupied){throw 'Porta HTTPS local ou de administracao ocupada; nenhuma instancia sera interrompida.'}
        Protect-Directory $base
        foreach($folder in @('data','logs')){
            $path=Join-Path $base $folder
            New-Item -ItemType Directory -Path $path -Force | Out-Null
            Protect-Directory $path $true
        }
        if(-not (Test-Path -LiteralPath $authFile)){
            $password=Random-Token 24
            # A senha entra pelo stdin, nunca nos argumentos do processo ou no log.
            $hash=($password | & $exe hash-password --algorithm argon2id).Trim()
            if($LASTEXITCODE -ne 0 -or -not $hash.StartsWith('$argon2id$')){throw 'Nao foi possivel gerar o hash da senha.'}
            [IO.File]::WriteAllText($authFile,(@{user=$WebUser;hash=$hash;session=(Random-Token)}|ConvertTo-Json),$utf8)
            $login=Join-Path $base 'login.txt'
            [IO.File]::WriteAllText($login,"Usuario: $WebUser`r`nSenha: $password`r`n`r`nEsta senha protege a entrada web. O pareamento do aplicativo continua obrigatorio.`r`n",$utf8)
            Protect-Secret $login
            $tokenFile=Join-Path $base 'duckdns-token.txt'
            [IO.File]::WriteAllText($tokenFile,'',$utf8)
            Protect-Secret $tokenFile
        }
        if($Action -eq 'Prepare' -or -not (Test-Path -LiteralPath $config)){Write-Config $false}
        if($Action -eq 'Prepare'){
            Write-Host "Configuracao preparada e validada. Credenciais: $base\login.txt."
            break
        }
        Require-Admin
        $binary='"'+$exe+'" run --config "'+$config+'" --adapter caddyfile'
        & sc.exe create $serviceName 'start=' 'delayed-auto' 'obj=' 'NT AUTHORITY\LocalService' 'binPath=' $binary 'DisplayName=' 'ModularChat - Caddy HTTPS'
        if($LASTEXITCODE -ne 0){throw 'Falha ao registrar o servico Caddy.'}
        & sc.exe failure $serviceName 'reset=' '86400' 'actions=' 'restart/10000/restart/30000/restart/60000' | Out-Null
        Start-Service -Name $serviceName
        Write-Host "Caddy instalado. Acesso local: https://localhost:$HttpsPort. Use a acao Trust para confiar no certificado local."
        Write-Host "Dados de entrada guardados em $base\login.txt."
    }
    'Activate' {
        Validate-HostOptions $true
        Require-Admin
        $dns=Resolve-DnsName $Domain -DnsOnly -ErrorAction Stop | Where-Object Type -in 'A','AAAA'
        if(-not $dns){throw 'O dominio ainda nao tem registros A ou AAAA; configuracao local preservada.'}
        if(-not (Get-Service -Name $serviceName -ErrorAction SilentlyContinue)){throw 'Instale o servico antes de ativar o dominio.'}
        $servicePid=(Get-CimInstance Win32_Service -Filter "Name='$serviceName'").ProcessId
        $occupied=Get-NetTCPConnection -State Listen -ErrorAction SilentlyContinue | Where-Object { $_.LocalPort -in 80,443 -and $_.OwningProcess -ne $servicePid }
        if($occupied){throw 'Porta 80 ou 443 usada por outro programa; nenhuma instancia sera interrompida.'}
        $ruleName='ModularChat-Caddy-Web'
        if(-not (Get-NetFirewallRule -Name $ruleName -ErrorAction SilentlyContinue)){
            New-NetFirewallRule -Name $ruleName -DisplayName 'ModularChat - Caddy HTTPS' -Direction Inbound -Action Allow -Protocol TCP -LocalPort 80,443 -Program $exe -Profile Private -RemoteAddress Any | Out-Null
        }
        Enable-NetFirewallRule -Name $ruleName | Out-Null
        $lanRule='ModularChat-Caddy-LAN-HTTPS'
        if($LanAddress -and -not (Get-NetFirewallRule -Name $lanRule -ErrorAction SilentlyContinue)){
            New-NetFirewallRule -Name $lanRule -DisplayName 'ModularChat - HTTPS rede local' -Direction Inbound -Action Allow -Protocol TCP -LocalPort $HttpsPort -Program $exe -Profile Private -RemoteAddress $LanSubnet | Out-Null
        }
        if($LanAddress){Enable-NetFirewallRule -Name $lanRule | Out-Null}
        Write-Config $true
        Start-Service -Name $serviceName -ErrorAction SilentlyContinue
        Write-Host "Dominio configurado: https://$Domain. O certificado publico depende de conexoes externas chegarem ao Caddy nas portas TCP 80 ou 443."
    }
    'Lan' {
        Require-Admin
        Validate-HostOptions $false
        if(-not $LanAddress){throw 'Informe -LanAddress e -LanSubnet para habilitar a rede local.'}
        Write-Config $false
        $lanRule='ModularChat-Caddy-LAN-HTTPS'
        if(-not (Get-NetFirewallRule -Name $lanRule -ErrorAction SilentlyContinue)){
            New-NetFirewallRule -Name $lanRule -DisplayName 'ModularChat - HTTPS rede local' -Direction Inbound -Action Allow -Protocol TCP -LocalPort $HttpsPort -Program $exe -Profile Private -RemoteAddress $LanSubnet | Out-Null
        }
        Enable-NetFirewallRule -Name $lanRule | Out-Null
        $webRule=Get-NetFirewallRule -Name 'ModularChat-Caddy-Web' -ErrorAction SilentlyContinue
        if($webRule){Disable-NetFirewallRule -InputObject $webRule | Out-Null}
        Start-Service -Name $serviceName -ErrorAction SilentlyContinue
        Write-Host "HTTPS habilitado na LAN: https://${LanAddress}:$HttpsPort."
    }
    'Local' {
        Require-Admin
        Write-Config $false
        foreach($name in @('ModularChat-Caddy-Web','ModularChat-Caddy-LAN-HTTPS')){
            $rule=Get-NetFirewallRule -Name $name -ErrorAction SilentlyContinue
            if($rule){Disable-NetFirewallRule -InputObject $rule | Out-Null}
        }
        $task=Get-ScheduledTask -TaskName 'ModularChat-DuckDNS' -ErrorAction SilentlyContinue
        if($task){Disable-ScheduledTask -InputObject $task | Out-Null}
        Write-Host 'Caddy somente no PC. Regras de entrada e atualizador DNS desativados; app e dados preservados.'
    }
    'Trust' {
        $root=Join-Path $base 'data\pki\authorities\local\root.crt'
        if(-not (Test-Path -LiteralPath $root)){throw 'O certificado local ainda nao foi gerado pelo servico.'}
        $cert=New-Object Security.Cryptography.X509Certificates.X509Certificate2($root)
        $store=New-Object Security.Cryptography.X509Certificates.X509Store('Root','CurrentUser')
        try{$store.Open('ReadWrite'); $store.Add($cert)}finally{$store.Close()}
        [IO.File]::WriteAllText((Join-Path $base 'trusted-root-thumbprint.txt'),$cert.Thumbprint,$utf8)
        Write-Host 'Certificado local confiado apenas para o usuario atual do Windows.'
    }
    'Login' {Get-Content -LiteralPath (Join-Path $base 'login.txt')}
    'PairCode' {
        $pair=Invoke-RestMethod -Uri "http://127.0.0.1:$BackendPort/api/pair/start" -Method Post -TimeoutSec 10
        Write-Host "Codigo de pareamento: $($pair.code). Valido por 5 minutos."
    }
    'Status' {
        Get-Service -Name $serviceName -ErrorAction SilentlyContinue | Select-Object Name,Status,StartType
        if(Test-Path -LiteralPath (Join-Path $base 'state.json')){Get-Content -LiteralPath (Join-Path $base 'state.json')}
        Write-Host "Endereco local: https://localhost:$HttpsPort. Credenciais: $base\login.txt."
    }
    'Stop' {Require-Admin; Stop-Service -Name $serviceName; Write-Host 'Somente o Caddy foi parado; o servidor ModularChat continua independente.'}
}

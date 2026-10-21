# Atualizador do DNS do host; nao grava token em argumentos, URLs impressas ou logs.
[CmdletBinding()]
param(
    [ValidateSet('Update','Enable','Disable')][string]$Action='Update',
    [string]$Subdomain='',
    [string]$Base=''
)
$ErrorActionPreference='Stop'
if(-not $Base){$Base=Join-Path (Split-Path $PSScriptRoot -Parent) 'hosting\caddy'}
$utf8=New-Object System.Text.UTF8Encoding($false)
if($Action -ne 'Disable' -and $Subdomain -notmatch '^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$'){throw 'Informe somente o subdominio DuckDNS usando -Subdomain.'}
function Update-Dns {
    $tokenFile=Join-Path $Base 'duckdns-token.txt'
    if(-not (Test-Path -LiteralPath $tokenFile)){throw "Guarde o token DuckDNS em $tokenFile."}
    $token=[IO.File]::ReadAllText($tokenFile).Trim()
    if($token -notmatch '^[a-zA-Z0-9-]{16,128}$'){throw 'Preencha o arquivo duckdns-token.txt com somente o token da conta DuckDNS.'}
    $stage='consultar IPv4 publico'
    try{
        [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
        $ip=(Invoke-RestMethod -Uri 'https://api.ipify.org?format=json' -TimeoutSec 15).ip
        $stage='validar IPv4 publico'
        [Net.IPAddress]$parsed=$null
        if(-not [Net.IPAddress]::TryParse($ip,[ref]$parsed) -or $parsed.AddressFamily -ne [Net.Sockets.AddressFamily]::InterNetwork){throw 'IPv4 invalido'}
        $stage='selecionar IPv6 publico'
        $v6=Get-NetIPAddress -AddressFamily IPv6 -ErrorAction Stop | Where-Object {
            $_.IPAddress -match '^[23][0-9a-f]{3}:' -and $_.AddressState -eq 'Preferred' -and $_.SuffixOrigin -ne 'Random' -and -not $_.SkipAsSource
        } | Select-Object -First 1 -ExpandProperty IPAddress
        $uri='https://www.duckdns.org/update?domains='+[Uri]::EscapeDataString($Subdomain)+'&token='+[Uri]::EscapeDataString($token)+'&ip='+[Uri]::EscapeDataString($ip)
        if($v6){$uri+='&ipv6='+[Uri]::EscapeDataString($v6)}
        $stage='atualizar DuckDNS'
        $reply=Invoke-WebRequest -Uri $uri -UseBasicParsing -TimeoutSec 20
        $stage='ler resposta DuckDNS'
        $body=if($reply.Content -is [byte[]]){[Text.Encoding]::UTF8.GetString($reply.Content)}else{[string]$reply.Content}
        $response=$body.Trim()
        $stage=if($response -eq 'KO'){'credencial ou subdominio recusado pelo DuckDNS'}else{'confirmar resposta DuckDNS'}
        if($response -ne 'OK'){throw 'DuckDNS recusou a atualizacao'}
        [IO.File]::WriteAllText((Join-Path $Base 'logs\duckdns-state.json'),(@{ok=$true;domain="$Subdomain.duckdns.org";ipv4=$ip;ipv6=$v6;updatedAt=(Get-Date).ToString('o')}|ConvertTo-Json),$utf8)
        Write-Host 'DuckDNS atualizado; enderecos temporarios IPv6 foram excluidos da escolha.'
    }catch{
        $safeError=$_.Exception.Message.Replace($token,'[TOKEN OMITIDO]') -replace 'https?://\S+','[URL OMITIDA]'
        [IO.File]::WriteAllText((Join-Path $Base 'logs\duckdns-state.json'),(@{ok=$false;stage=$stage;errorKind=$_.Exception.GetType().FullName;error=$safeError;updatedAt=(Get-Date).ToString('o')}|ConvertTo-Json),$utf8)
        throw 'Falha ao atualizar DuckDNS. O token foi omitido do diagnostico; confira a conta e a conectividade.'
    }
}
switch($Action){
    'Update' {Update-Dns}
    'Enable' {
        $identity=[Security.Principal.WindowsIdentity]::GetCurrent()
        if(-not (New-Object Security.Principal.WindowsPrincipal($identity)).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)){throw 'Abra o PowerShell como Administrador para ativar o atualizador.'}
        if(Get-ScheduledTask -TaskName 'ModularChat-DuckDNS' -ErrorAction SilentlyContinue){throw 'O atualizador ja esta instalado; nenhuma tarefa existente sera substituida.'}
        Update-Dns
        $installed=Join-Path $Base 'duckdns-update.ps1'
        Copy-Item -LiteralPath $PSCommandPath -Destination $installed -Force
        $arguments='-NoProfile -NonInteractive -WindowStyle Hidden -ExecutionPolicy Bypass -File "'+$installed+'" -Action Update -Subdomain '+$Subdomain+' -Base "'+$Base+'"'
        $taskAction=New-ScheduledTaskAction -Execute (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') -Argument $arguments
        $trigger=New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(5) -RepetitionInterval (New-TimeSpan -Minutes 5)
        $principal=New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
        $settings=New-ScheduledTaskSettingsSet -Hidden -StartWhenAvailable -ExecutionTimeLimit (New-TimeSpan -Minutes 2) -MultipleInstances IgnoreNew -AllowStartIfOnBatteries -DontStopIfGoingOnBatteries
        Register-ScheduledTask -TaskName 'ModularChat-DuckDNS' -Action $taskAction -Trigger $trigger -Principal $principal -Settings $settings -Description 'Atualiza os registros A/AAAA do host privado ModularChat no DuckDNS.' | Out-Null
        Write-Host 'Atualizacao de DNS a cada 5 minutos ativada em segundo plano.'
    }
    'Disable' {
        $task=Get-ScheduledTask -TaskName 'ModularChat-DuckDNS' -ErrorAction SilentlyContinue
        if($task){Disable-ScheduledTask -InputObject $task | Out-Null}
        Write-Host 'Atualizador DuckDNS desativado. Os registros existentes foram preservados.'
    }
}

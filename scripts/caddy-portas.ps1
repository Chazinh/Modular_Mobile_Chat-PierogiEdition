# Encaminhamento IPv4 por UPnP, limitado a 80/443; nao altera regras alheias.
[CmdletBinding()]
param(
    [ValidateSet('Apply','Status','Remove')][string]$Action='Status',
    [string]$Base=(Join-Path $env:USERPROFILE 'ModularChat-local\caddy')
)
$ErrorActionPreference='Stop'
$base=[IO.Path]::GetFullPath($Base)
$manifest=Join-Path $base 'port-mappings.json'
$nat=New-Object -ComObject HNetCfg.NATUPnP
$maps=$nat.StaticPortMappingCollection
if($null -eq $maps){throw 'O roteador nao oferece UPnP. Configure TCP 80/443 manualmente no painel dele.'}
$description='ModularChat Caddy HTTPS'
$current=@($maps)
if($Action -eq 'Status'){
    $current|Where-Object {$_.ExternalPort -in 80,443}|Select-Object ExternalPort,InternalPort,InternalClient,ExternalIPAddress,Protocol,Description
    return
}
if($Action -eq 'Remove'){
    if(-not (Test-Path -LiteralPath $manifest)){throw 'Manifesto de regras desta tarefa nao encontrado; nenhuma regra sera removida.'}
    $record=Get-Content -LiteralPath $manifest -Raw|ConvertFrom-Json
    if($record.description -ne $description -or @($record.addedPorts | Where-Object { $_ -notin 80,443 }).Count){throw 'Manifesto invalido; nenhuma regra sera removida.'}
    foreach($port in $record.addedPorts){
        $mapping=$current|Where-Object {$_.ExternalPort -eq $port -and $_.Protocol -eq 'TCP'}|Select-Object -First 1
        if($mapping -and $mapping.Description -eq $description -and $mapping.InternalClient -eq $record.internalClient -and $mapping.InternalPort -eq $port){$maps.Remove([int]$port,'TCP')}
    }
    Write-Host 'Somente os encaminhamentos registrados por esta configuracao foram removidos.'
    return
}
$gateway=Get-NetRoute -AddressFamily IPv4 -DestinationPrefix '0.0.0.0/0'|Sort-Object RouteMetric|Select-Object -First 1
$ip=Get-NetIPAddress -InterfaceIndex $gateway.InterfaceIndex -AddressFamily IPv4|Where-Object {$_.IPAddress -notlike '169.254.*' -and $_.IPAddress -notlike '127.*'}|Select-Object -First 1 -ExpandProperty IPAddress
if(-not $ip){throw 'IPv4 local nao encontrado; nenhuma regra sera adicionada.'}
foreach($port in @(80,443)){
    $existing=$current|Where-Object {$_.ExternalPort -eq $port -and $_.Protocol -eq 'TCP'}|Select-Object -First 1
    if($existing -and ($existing.InternalClient -ne $ip -or $existing.InternalPort -ne $port)){
        throw "TCP $port ja pertence a outro destino. As regras existentes foram preservadas."
    }
}
$added=New-Object 'System.Collections.Generic.List[int]'
$owned=@()
$previousManifest=if(Test-Path -LiteralPath $manifest){[IO.File]::ReadAllText($manifest)}else{$null}
if($previousManifest){
    $previous=$previousManifest | ConvertFrom-Json
    if($previous.description -ne $description -or $previous.internalClient -ne $ip -or @($previous.addedPorts | Where-Object { $_ -notin 80,443 }).Count){throw 'O manifesto pertence a outra configuracao; nenhuma regra sera adicionada.'}
    $owned=@($previous.addedPorts)
}
$utf8=New-Object Text.UTF8Encoding($false)
try{
    foreach($port in @(80,443)){
        if(-not ($current|Where-Object {$_.ExternalPort -eq $port -and $_.Protocol -eq 'TCP'})){
            $mapping=$maps.Add($port,'TCP',$port,$ip,$true,$description)
            $added.Add($port)
            $allOwned=@(@($owned)+@($added.ToArray()) | Select-Object -Unique)
            [IO.File]::WriteAllText($manifest,(@{description=$description;internalClient=$ip;addedPorts=$allOwned;externalIPv4=$mapping.ExternalIPAddress;updatedAt=(Get-Date).ToString('o')}|ConvertTo-Json),$utf8)
        }
    }
}catch{
    foreach($port in $added){
        $mapping=@($maps)|Where-Object {$_.ExternalPort -eq $port -and $_.Protocol -eq 'TCP'}|Select-Object -First 1
        if($mapping -and $mapping.Description -eq $description -and $mapping.InternalClient -eq $ip){$maps.Remove($port,'TCP')}
    }
    if($previousManifest){[IO.File]::WriteAllText($manifest,$previousManifest,$utf8)}elseif(Test-Path -LiteralPath $manifest){Remove-Item -LiteralPath $manifest}
    throw 'O roteador recusou o encaminhamento. As novas regras deste lote foram desfeitas; configure as portas no painel.'
}
@($maps)|Where-Object {$_.ExternalPort -in 80,443 -and $_.Protocol -eq 'TCP'}|Select-Object ExternalPort,InternalPort,InternalClient,ExternalIPAddress,Protocol,Description|ConvertTo-Json -Depth 3

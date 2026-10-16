# Generate optional Android trust resources; no private keys and no trust-store changes.
[CmdletBinding()]
param(
    [Parameter(Mandatory=$true)][string]$HostName,
    [Parameter(Mandatory=$true)][string]$CertificatePath,
    [string]$OutputDirectory=(Join-Path (Split-Path $PSScriptRoot -Parent) 'client\android\host-trust')
)
$ErrorActionPreference='Stop'
if($HostName -notmatch '^[a-zA-Z0-9.-]+$' -or $HostName.StartsWith('.') -or $HostName.EndsWith('.') -or $HostName.Contains('..')){
    throw 'Informe um nome DNS ou IPv4, sem protocolo, porta ou caminho.'
}
$certificateFile=(Resolve-Path -LiteralPath $CertificatePath).Path
$bytes=[IO.File]::ReadAllBytes($certificateFile)
$text=[Text.Encoding]::UTF8.GetString($bytes)
if($text -match 'PRIVATE KEY' -or [IO.Path]::GetExtension($certificateFile) -in '.pfx','.p12'){
    throw 'Use somente o certificado publico da CA, nunca uma chave privada ou arquivo PKCS#12.'
}
$certificate=New-Object Security.Cryptography.X509Certificates.X509Certificate2 -ArgumentList (,$bytes)
try {
    $constraints=$certificate.Extensions | Where-Object { $_.Oid.Value -eq '2.5.29.19' } | Select-Object -First 1
    if(-not $constraints -or -not $constraints.CertificateAuthority -or $certificate.HasPrivateKey){throw 'O arquivo precisa conter apenas um certificado publico de autoridade certificadora.'}
    if($certificate.NotAfter -le (Get-Date) -or $certificate.NotBefore -gt (Get-Date)){throw 'O certificado da CA nao esta dentro da validade.'}
    $publicBytes=$certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert)
}finally{$certificate.Dispose()}
$destination=[IO.Path]::GetFullPath($OutputDirectory)
foreach($folder in @('xml','raw')){New-Item -ItemType Directory -Path (Join-Path $destination $folder) -Force | Out-Null}
$utf8=New-Object Text.UTF8Encoding($false)
$hostCleartext='false'
$hostIp=$null
if([Net.IPAddress]::TryParse($HostName,[ref]$hostIp) -and $hostIp.AddressFamily -eq [Net.Sockets.AddressFamily]::InterNetwork){
    $octets=$hostIp.GetAddressBytes()
    if($octets[0] -in 10,127 -or ($octets[0] -eq 192 -and $octets[1] -eq 168) -or
       ($octets[0] -eq 172 -and $octets[1] -ge 16 -and $octets[1] -le 31)){$hostCleartext='true'}
}elseif($HostName -eq 'localhost' -or $HostName.EndsWith('.local',[StringComparison]::OrdinalIgnoreCase)){$hostCleartext='true'}
$xml=@"
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <!-- LAN addresses can change; the client restricts HTTP to local addresses. -->
    <base-config cleartextTrafficPermitted="true">
        <trust-anchors><certificates src="system" /><certificates src="user" /></trust-anchors>
    </base-config>
    <domain-config cleartextTrafficPermitted="$hostCleartext">
        <domain includeSubdomains="false">$HostName</domain>
        <trust-anchors><certificates src="@raw/modularchat_host_ca" /></trust-anchors>
    </domain-config>
    <domain-config cleartextTrafficPermitted="true">
        <domain includeSubdomains="false">localhost</domain>
        <domain includeSubdomains="false">127.0.0.1</domain>
    </domain-config>
</network-security-config>
"@
[IO.File]::WriteAllBytes((Join-Path $destination 'raw\modularchat_host_ca.crt'),$publicBytes)
[IO.File]::WriteAllText((Join-Path $destination 'xml\modularchat_host_security_config.xml'),$xml,$utf8)
Write-Host "Recursos locais gerados. Defina MC_ANDROID_HOST_TRUST_DIR para $destination antes do build Android."

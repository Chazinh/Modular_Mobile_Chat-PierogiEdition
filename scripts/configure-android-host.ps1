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
$xml=@"
<?xml version="1.0" encoding="utf-8"?>
<network-security-config>
    <base-config cleartextTrafficPermitted="false">
        <trust-anchors><certificates src="system" /></trust-anchors>
    </base-config>
    <domain-config cleartextTrafficPermitted="false">
        <domain includeSubdomains="false">$HostName</domain>
        <trust-anchors><certificates src="@raw/modularchat_host_ca" /></trust-anchors>
    </domain-config>
    <domain-config cleartextTrafficPermitted="true">
        <domain includeSubdomains="false">localhost</domain>
    </domain-config>
</network-security-config>
"@
[IO.File]::WriteAllBytes((Join-Path $destination 'raw\modularchat_host_ca.crt'),$publicBytes)
[IO.File]::WriteAllText((Join-Path $destination 'xml\modularchat_host_security_config.xml'),$xml,$utf8)
Write-Host "Recursos locais gerados. Defina MC_ANDROID_HOST_TRUST_DIR para $destination antes do build Android."

# Isolated checks: parse scripts, invoke only pure option validation, generate a fictional public CA.
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
$files=@('scripts\caddy-host.ps1','scripts\duckdns-host.ps1','scripts\caddy-portas.ps1','scripts\configure-android-host.ps1','start-server.ps1')
$hostAst=$null
foreach($relative in $files){
    $tokens=$null;$parseErrors=$null
    $ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $root $relative),[ref]$tokens,[ref]$parseErrors)
    if($parseErrors.Count){throw "Erro de sintaxe em ${relative}: $($parseErrors[0].Message)"}
    if($relative -eq 'scripts\caddy-host.ps1'){$hostAst=$ast}
}
$definition=$hostAst.Find({param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Validate-HostOptions'},$true)
Invoke-Expression $definition.Extent.Text
$BackendPort=3000;$HttpsPort=8443;$AdminPort=2019
$Domain='chat.example.com';$LanAddress='192.168.0.10';$LanSubnet='192.168.0.0/24'
Validate-HostOptions $true
function Expect-Rejection([scriptblock]$operation){
    $rejected=$false
    try{& $operation}catch{$rejected=$true}
    if(-not $rejected){throw 'Uma entrada invalida foi aceita.'}
}
Expect-Rejection { $Domain='https://chat.example.com/path';Validate-HostOptions $true }
Expect-Rejection { $AdminPort=3000;Validate-HostOptions $true }
Expect-Rejection { $LanAddress='192.0.2.10';Validate-HostOptions $true }
Expect-Rejection { $LanSubnet='192.168.0.0/8';Validate-HostOptions $true }
Expect-Rejection { $LanSubnet='192.168.1.0/24';Validate-HostOptions $true }
Expect-Rejection { $LanSubnet='999.168.0.0/24';Validate-HostOptions $true }
Expect-Rejection { $LanAddress='';Validate-HostOptions $true }

$tempRoot=[IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$testDirectory=[IO.Path]::GetFullPath((Join-Path $tempRoot ('mc-host-config-'+[guid]::NewGuid().ToString('N'))))
if(-not $testDirectory.StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase)){throw 'Diretorio de teste invalido.'}
New-Item -ItemType Directory -Path $testDirectory | Out-Null
$rsa=[Security.Cryptography.RSACng]::new(2048)
$certificate=$null
try {
    $request=[Security.Cryptography.X509Certificates.CertificateRequest]::new('CN=ModularChat Fixture CA',$rsa,[Security.Cryptography.HashAlgorithmName]::SHA256,[Security.Cryptography.RSASignaturePadding]::Pkcs1)
    $request.CertificateExtensions.Add([Security.Cryptography.X509Certificates.X509BasicConstraintsExtension]::new($true,$false,0,$true))
    $certificate=$request.CreateSelfSigned([DateTimeOffset]::UtcNow.AddDays(-1),[DateTimeOffset]::UtcNow.AddDays(30))
    $publicFile=Join-Path $testDirectory 'fixture-ca.crt'
    [IO.File]::WriteAllBytes($publicFile,$certificate.Export([Security.Cryptography.X509Certificates.X509ContentType]::Cert))
    $output=Join-Path $testDirectory 'resources'
    & (Join-Path $root 'scripts\configure-android-host.ps1') -HostName '192.0.2.10' -CertificatePath $publicFile -OutputDirectory $output | Out-Null
    [xml]$xml=Get-Content -LiteralPath (Join-Path $output 'xml\modularchat_host_security_config.xml') -Raw
    if($xml.'network-security-config'.'base-config'.cleartextTrafficPermitted -ne 'true'){throw 'LAN HTTP dinamica foi bloqueada.'}
    $scoped=$xml.'network-security-config'.'domain-config' | Where-Object { $_.domain.'#text' -eq '192.0.2.10' }
    if($scoped.cleartextTrafficPermitted -ne 'false'){throw 'HTTP do host publico nao foi bloqueado.'}
    if(-not $scoped -or $scoped.domain.includeSubdomains -ne 'false' -or $scoped.'trust-anchors'.certificates.src -ne '@raw/modularchat_host_ca'){throw 'Confianca nao ficou restrita ao host informado.'}
    if(@(Get-ChildItem -LiteralPath $output -Recurse -File).Count -ne 2){throw 'Recursos inesperados foram gerados.'}
    foreach($lanHost in @('10.229.240.173','192.168.1.100','172.16.0.2','pc.local')){
        & (Join-Path $root 'scripts\configure-android-host.ps1') -HostName $lanHost -CertificatePath $publicFile -OutputDirectory $output | Out-Null
        [xml]$lanXml=Get-Content -LiteralPath (Join-Path $output 'xml\modularchat_host_security_config.xml') -Raw
        $lanScope=$lanXml.'network-security-config'.'domain-config' | Where-Object { $_.domain.'#text' -eq $lanHost }
        if($lanScope.cleartextTrafficPermitted -ne 'true'){throw 'HTTP do host LAN nao foi permitido.'}
    }
    $privateFile=Join-Path $testDirectory 'private.pem'
    [IO.File]::WriteAllText($privateFile,'-----BEGIN PRIVATE KEY-----')
    Expect-Rejection { & (Join-Path $root 'scripts\configure-android-host.ps1') -HostName '192.0.2.10' -CertificatePath $privateFile -OutputDirectory (Join-Path $testDirectory 'rejected') }
    Expect-Rejection { & (Join-Path $root 'scripts\configure-android-host.ps1') -HostName 'https://192.0.2.10/path' -CertificatePath $publicFile -OutputDirectory (Join-Path $testDirectory 'rejected') }
    if(Test-Path -LiteralPath (Join-Path $testDirectory 'rejected')){throw 'Entrada recusada gerou recursos.'}
    Write-Output 'Hosting: 5 scripts parsed; host/subnet/port restrictions and public-CA Android resources verified.'
}finally{
    if($certificate){$certificate.Dispose()}
    $rsa.Dispose()
    if(-not $testDirectory.StartsWith($tempRoot,[StringComparison]::OrdinalIgnoreCase)){throw 'Limpeza fora do diretorio temporario.'}
    Remove-Item -LiteralPath $testDirectory -Recurse -Force
}

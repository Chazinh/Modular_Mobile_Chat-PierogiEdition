$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot -Parent
$fixtureState = [pscustomobject]@{Exists=$false;Calls=[Collections.Generic.List[object]]::new()}
function Get-NetFirewallRule { [CmdletBinding()]param($Name) if ($fixtureState.Exists) { [pscustomobject]@{Name=$Name} } }
function Get-NetConnectionProfile { [CmdletBinding()]param() [pscustomobject]@{InterfaceAlias='Fixture Wi-Fi';Name='Fixture';NetworkCategory='Public'} }
function New-NetFirewallRule {
    [CmdletBinding()]param($Name,$DisplayName,$Direction,$Protocol,$LocalPort,$Action,$Profile,$RemoteAddress)
    $fixtureState.Calls.Add([pscustomobject]@{Operation='create';Name=$Name;Direction=$Direction;Protocol=$Protocol;Port=$LocalPort;Action=$Action;Profile=$Profile;Scope=$RemoteAddress})
}
function Set-NetFirewallRule {
    [CmdletBinding()]param($Name,$Enabled,$Direction,$Protocol,$LocalPort,$Action,$Profile,$RemoteAddress)
    if ($Enabled -ne $true) { throw 'The existing rule must be enabled.' }
    $fixtureState.Calls.Add([pscustomobject]@{Operation='update';Name=$Name;Direction=$Direction;Protocol=$Protocol;Port=$LocalPort;Action=$Action;Profile=$Profile;Scope=$RemoteAddress})
}
& (Join-Path $root 'open-firewall.ps1') -Port 3000 | Out-Null
$fixtureState.Exists = $true
& (Join-Path $root 'open-firewall.ps1') -Port 3000 | Out-Null
if ($fixtureState.Calls.Count -ne 2 -or $fixtureState.Calls[0].Operation -ne 'create' -or $fixtureState.Calls[1].Operation -ne 'update') { throw 'Repeated invocation must update the same rule.' }
foreach ($call in $fixtureState.Calls) {
    if ($call.Name -ne 'ModularChat-LAN-TCP-3000' -or $call.Direction -ne 'Inbound' -or $call.Protocol -ne 'TCP' -or $call.Port -ne 3000 -or $call.Action -ne 'Allow' -or $call.Scope -ne 'LocalSubnet') { throw 'Unexpected firewall scope.' }
    if ((@($call.Profile | Sort-Object) -join ',') -ne 'Private,Public') { throw 'Both home Wi-Fi profile categories must be supported.' }
}
$rejected = $false
try { & (Join-Path $root 'open-firewall.ps1') -Port 0 } catch { $rejected = $true }
if (-not $rejected -or $fixtureState.Calls.Count -ne 2) { throw 'Invalid port must be rejected before changing any rule.' }
'Firewall fixture passed: create/update, Private/Public, TCP port 3000, LocalSubnet, invalid port rejected.'

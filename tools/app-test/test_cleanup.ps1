#requires -Version 7.2
# Synthetic cleanup-policy fixtures. Never loads the runner body or invokes adb.
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$tokens=$null;$parseErrors=$null
$runner=Join-Path $PSScriptRoot 'run_android.ps1'
$ast=[Management.Automation.Language.Parser]::ParseFile($runner,[ref]$tokens,[ref]$parseErrors)
if($parseErrors.Count -ne 0){throw 'HIL runner syntax refused.'}
$cleanup=@($ast.FindAll({param($node)
    $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Cleanup-Packages'
},$true))
if($cleanup.Count -ne 1){throw 'Exact cleanup function unavailable.'}
. ([scriptblock]::Create($cleanup[0].Extent.Text))

function Assert-Fixture([bool]$Condition,[string]$Code){if(-not $Condition){throw "Cleanup fixture refused: $Code"}}
function Package-Path([string]$Package){
    Assert-Fixture ($Package -ceq $script:packages[0]) 'package-scope'
    $script:inventoryReads++
    if($script:present){return '/synthetic/base.apk'}
    return $null
}
function Owned-Package([string]$Package,[string]$ExpectedHash){
    Assert-Fixture ($Package -ceq $script:packages[0] -and $ExpectedHash -ceq ('a'*64)) 'exact-hash-request'
    $script:bindingReads++
    return $script:matching
}
function Adb([string]$Name,[string[]]$Arguments,[int]$Seconds=30){
    $script:actions+=$Name
    if($Name -ceq 'app-hil-stop-owned-package'){
        Assert-Fixture (($Arguments -join ' ') -ceq ('shell am force-stop --user 0 '+$script:packages[0])) 'stop-owner-only'
        return [pscustomobject]@{Lines=@()}
    }
    Assert-Fixture ($Name -ceq 'app-hil-remove-owned-package') 'unknown-adb-action'
    Assert-Fixture (($Arguments -join ' ') -ceq ('uninstall --user 0 '+$script:packages[0])) 'remove-owner-only'
    $script:present=$false
    return [pscustomobject]@{Lines=@('Success')}
}

$cases=@(
    @{name='no-attempt';attempted=$false;installed=$false;absent=$true;present=$false;matching=$true;ok=$true;pending=$false;reads=0;bindings=0;actions=0},
    @{name='lost-success-looks-absent';attempted=$true;installed=$false;absent=$true;present=$false;matching=$true;ok=$false;pending=$true;reads=0;bindings=0;actions=0},
    @{name='lost-success-looks-present';attempted=$true;installed=$false;absent=$true;present=$true;matching=$true;ok=$false;pending=$true;reads=0;bindings=0;actions=0},
    @{name='unknown-install-no-preabsence';attempted=$true;installed=$false;absent=$false;present=$false;matching=$true;ok=$false;pending=$true;reads=0;bindings=0;actions=0},
    @{name='confirmed-success-already-absent';attempted=$true;installed=$true;absent=$true;present=$false;matching=$true;ok=$true;pending=$false;reads=1;bindings=0;actions=0},
    @{name='confirmed-success-owned-bytes';attempted=$true;installed=$true;absent=$true;present=$true;matching=$true;ok=$true;pending=$false;reads=2;bindings=1;actions=2},
    @{name='confirmed-success-changed-bytes';attempted=$true;installed=$true;absent=$true;present=$true;matching=$false;ok=$false;pending=$false;reads=1;bindings=1;actions=0},
    @{name='confirmed-success-no-preabsence';attempted=$true;installed=$true;absent=$false;present=$true;matching=$true;ok=$false;pending=$false;reads=0;bindings=0;actions=0},
    @{name='pending-test-preserves-known-main-cleanup';attempted=$true;installed=$true;absent=$true;present=$true;matching=$true;ok=$false;pending=$true;reads=2;bindings=1;actions=2;pendingTest=$true}
)
$script:packages=@('com.visualworkbench.android.hil','com.visualworkbench.android.hil.test')
foreach($case in $cases){
    $script:attempted=@{};$script:installed=@{};$script:absent=@{}
    $script:receipt=@{cleanup=@{device_installer_pending=$false}}
    $script:artifacts=[pscustomobject]@{artifacts=[pscustomobject]@{main=[pscustomobject]@{sha256=('a'*64)}}}
    if($case.attempted){$script:attempted[$packages[0]]=$true}
    if($case.installed){$script:installed[$packages[0]]=$true}
    if($case.ContainsKey('pendingTest')){$script:attempted[$packages[1]]=$true}
    $script:absent[$packages[0]]=$case.absent
    $script:present=$case.present;$script:matching=$case.matching
    $script:inventoryReads=0;$script:bindingReads=0;$script:actions=@()
    $result=Cleanup-Packages
    Assert-Fixture ($result -eq $case.ok) ($case.name+'-cleanup')
    Assert-Fixture ($receipt.cleanup.device_installer_pending -eq $case.pending) ($case.name+'-pending')
    Assert-Fixture ($inventoryReads -eq $case.reads -and $bindingReads -eq $case.bindings -and $actions.Count -eq $case.actions) ($case.name+'-no-unowned-actions')
}
Write-Output 'APP_HIL_CLEANUP_FIXTURES_PASS: nine synthetic failure/success ownership paths; no adb/device execution'

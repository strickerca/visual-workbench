#requires -Version 7.2
[CmdletBinding()]
param(
    [Parameter(Mandatory)][switch]$Execute,
    [Parameter(Mandatory)][switch]$OwnerReady,
    [Parameter(Mandatory)][string]$ArtifactManifest,
    [Parameter(Mandatory)][ValidatePattern('^[a-f0-9]{64}$')][string]$ArtifactManifestSha256,
    [Parameter(Mandatory)][int]$UsbInterfaceIndex,
    [Parameter(Mandatory)][string]$UsbDeviceInstanceSha256,
    [Parameter(Mandatory)][string]$PcUsbAddress,
    [Parameter(Mandatory)][string]$PhoneUsbAddress,
    [Parameter(Mandatory)][int]$Port,
    [ValidateSet("balanced","pause-held","background-held","disconnect-held")][string]$InputScenario="balanced"
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $Execute -or -not $OwnerReady -or -not $IsWindows){throw 'Explicit retained owner execution required.'}
$root=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
Import-Module (Join-Path $root 'tools/process.psm1')
. (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
. (Join-Path $PSScriptRoot 'input_owner_startup.ps1')
Initialize-M4InputReaders
$nonce=[guid]::NewGuid().ToString('N')
$gateDirectory=Join-Path ([IO.Path]::GetTempPath()) ('VisualWorkbench-input-parent-'+$nonce)
[void][IO.Directory]::CreateDirectory($gateDirectory)
$gate=Join-Path $gateDirectory 'assigned.json'
$process=[Diagnostics.Process]::new()
$process.StartInfo=[Diagnostics.ProcessStartInfo]::new((Join-Path $PSHOME 'pwsh.exe'))
$process.StartInfo.UseShellExecute=$false;$process.StartInfo.CreateNoWindow=$true
$process.StartInfo.WorkingDirectory=$root
$process.StartInfo.RedirectStandardOutput=$true;$process.StartInfo.RedirectStandardError=$true
foreach($arg in @('-NoProfile','-File',(Join-Path $PSScriptRoot 'integration.ps1'),'-Execute','-OwnerReady','-ControllerInput','-InputScenario',$InputScenario,'-RetainedInputGate',$gate,'-RetainedInputNonce',$nonce)){
    [void]$process.StartInfo.ArgumentList.Add($arg)
}
foreach($name in @('ArtifactManifest','ArtifactManifestSha256','UsbInterfaceIndex','UsbDeviceInstanceSha256','PcUsbAddress','PhoneUsbAddress','Port')){
    [void]$process.StartInfo.ArgumentList.Add('-'+$name);[void]$process.StartInfo.ArgumentList.Add([string]$PSBoundParameters[$name])
}
foreach($key in @($process.StartInfo.Environment.Keys)){
    if($key -match '(?i)(TOKEN|SECRET|PASSWORD|API_KEY|PRIVATE_KEY|CREDENTIAL|AUTH_KEY)'){[void]$process.StartInfo.Environment.Remove($key)}
}
$output=[VwInputOutputR57]::new(@($env:USERPROFILE,$root,$gateDirectory,$PcUsbAddress,$PhoneUsbAddress))
$job=[VwProcessJobV4]::new();$entry=New-M4InputChild 'retained-runner' $process $job $output;$complete=$false;$exitCode=1
function Test-RunnerSettlement {
    if($entry.GateState -ceq 'Absent'){return $true}
    $receiptPath=Join-Path $root ('.local/remote-normal-integration-'+$nonce+'.json')
    try{
        $item=Get-Item -LiteralPath $receiptPath
        if($item.Length -le 0 -or $item.Length -gt 262144){return $false}
        while($null -ne $item){
            if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){return $false}
            $item=if($item -is [IO.DirectoryInfo]){$item.Parent}else{$item.Directory}
        }
        $file=[IO.File]::Open($receiptPath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
        try{
            $reader=[IO.StreamReader]::new($file)
            try{$receipt=$reader.ReadToEnd()|ConvertFrom-Json;return (Test-M4InputRunnerRetired $receipt $nonce)}finally{$reader.Dispose()}
        }finally{$file.Dispose()}
    }catch{return $false}
}
function Wait-RetainedInputOwner {
    param($ProgressClock=[Diagnostics.Stopwatch]::StartNew())
    $watch=$ProgressClock;$next=0
    while($true){
        $line=$null
        while($output.Lines.TryDequeue([ref]$line)){
            # Only source-owned bounded progress enters the console. Runner owns
            # the complete sanitized receipt; arbitrary native output stays private.
            if($line -cmatch '^(?:Remote integration phase=[a-zA-Z0-9_-]+ waiting=[0-9]+s|Real remote integration: phase=[a-zA-Z0-9_-]{1,64} elapsed=[0-9]{1,9}s|Real remote integration stopped in phase=[a-zA-Z0-9_-]{1,64}\. Private details and addresses are withheld\.|Remote host failure: code=(?:none|closed|pending|partial_input|backpressure|timeout|transport|worker|unavailable|invalid|authentication|session_other|cancelled|argument|state|assertion|io|other)|Remote host retirement: first=(?:none|closed|pending|partial_input|backpressure|timeout|transport|worker|unavailable|invalid|authentication|session_other|cancelled|argument|state|assertion|io|other) current=(?:none|closed|pending|partial_input|backpressure|timeout|transport|worker|unavailable|invalid|authentication|session_other|cancelled|argument|state|assertion|io|other) attempts=[0-9]{1,19} failures=[0-9]{1,19}|Pending actual input/native/phone retirement; owned destination and Jobs remain alive|Retaining actual inputful Job/IO until exit and joins|Text-only integration receipt: remote-normal-integration-[a-f0-9]{32}\.json; status=[a-z]+)$'){try{Write-Host $line}catch{}}
        }
        try{
            if((Test-M4InputChildSettled $entry) -and (Test-RunnerSettlement)){return}
        }catch{} # An unknown query never proves settlement.
        if($watch.Elapsed.TotalSeconds -ge $next){try{Write-Host 'Retained input owner alive; awaiting actual child Job and output settlement'}catch{};$next+=2}
        Start-Sleep -Milliseconds 20
    }
}
try{
    Start-M4InputChild $entry
    $self=[Diagnostics.Process]::GetCurrentProcess()
    try{
        $proof=[ordered]@{schema='M4_INPUT_PARENT_V1';run_id=$nonce;nonce=$nonce;assigned=$true;child_pid=$process.Id;child_created=[string]$process.StartTime.ToUniversalTime().ToFileTimeUtc();parent_pid=$PID;parent_created=[string]$self.StartTime.ToUniversalTime().ToFileTimeUtc()}
        $bytes=[Text.Encoding]::UTF8.GetBytes(($proof|ConvertTo-Json -Compress))
        Publish-M4InputGate $entry $gate $bytes
    }finally{$self.Dispose()}
    Wait-RetainedInputOwner;$exitCode=$process.ExitCode;$complete=$true
}finally{
    if(-not $complete){
        # A published or ambiguous gate requires the inner correlated receipt,
        # including a valid gate rejected by the inner owner before startup.
        Complete-M4InputOuterOwner $entry {Wait-RetainedInputOwner};$complete=$true
    }
    if($complete){$job.Dispose();$process.Dispose()}
    if($complete){
        foreach($name in @('assigned.json','assigned.json.pending')){
            $candidate=[IO.Path]::Combine([IO.Path]::GetFullPath($gateDirectory),$name)
            if([IO.Path]::GetDirectoryName($candidate) -cne [IO.Path]::GetFullPath($gateDirectory)){throw 'Gate cleanup containment failed.'}
            if(Test-Path -LiteralPath $candidate){Remove-Item -LiteralPath $candidate}
        }
        if([IO.Directory]::GetFileSystemEntries($gateDirectory).Length -eq 0){[IO.Directory]::Delete($gateDirectory)}
    }
}

exit $exitCode

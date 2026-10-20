#requires -Version 7.2
# Run locally on the explicitly selected S23 route with existing built artifacts.
# This parent owns the fresh Paint document until real remote input settlement.
[CmdletBinding()]
param(
    [Parameter(Mandatory)][switch]$Execute,
    [Parameter(Mandatory)][switch]$OwnerReady,
    [Parameter(Mandatory)][string]$PaintPath,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$PaintSha256,
    [Parameter(Mandatory)][string]$ProbeRunnerPath,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$ProbeRunnerSha256,
    [Parameter(Mandatory)][string]$ProbePath,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$ProbeSha256,
    [Parameter(Mandatory)][string]$FrameRunnerPath,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$FrameRunnerSha256,
    [Parameter(Mandatory)][string]$CaptureHelperPath,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$CaptureHelperSha256,
    [Parameter(Mandatory)][string]$ArtifactManifest,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$ArtifactManifestSha256,
    [Parameter(Mandatory)][ValidateRange(1,2147483647)][int]$UsbInterfaceIndex,
    [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$UsbDeviceInstanceSha256,
    [Parameter(Mandatory)][string]$PcUsbAddress,
    [Parameter(Mandatory)][string]$PhoneUsbAddress,
    [Parameter(Mandatory)][ValidateRange(1024,65535)][int]$Port,
    [string]$PythonPath='python',
    [ValidateRange(0,60)][int]$InspectSeconds=10,
    [ValidateRange(30,300)][int]$WorkTimeoutSeconds=90
)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
if(-not $Execute -or -not $OwnerReady -or -not $IsWindows){throw 'Explicit local Windows owned-editor execution required'}
$repo=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$run=[Guid]::NewGuid().ToString('N')
$pins=[Collections.Generic.List[IDisposable]]::new()
$directoryPins=[Collections.Generic.HashSet[string]]::new([StringComparer]::OrdinalIgnoreCase)
$children=[Collections.Generic.List[object]]::new()
$editor=$null;$remoteSettlement=$null;$privateRoot=$null;$fixtureRoot=$null
$canvasFiles=[Collections.Generic.List[string]]::new()
$cursorSamples=[Collections.Generic.List[object]]::new();$cursorAnchor=$null
$ownedDirectories=[Collections.Generic.List[string]]::new()
$phase='artifact_binding';$work=[Diagnostics.Stopwatch]::StartNew()
$diagnostic=[ordered]@{checkpoint='artifact_binding';refusal_code='unknown';helper_role='none';helper_exit_code='unknown';helper_timed_out='unknown';helper_output_limit='unknown';native_error='unknown';native_capture_refusal='unknown'}
$receipt=[ordered]@{schema=1;kind='owned-paint-phone-visible-drawing';run_id=$run;status='failed';failure_phase=$phase;artifact_manifest_sha256=$ArtifactManifestSha256;source_count=0;paint_sha256=$PaintSha256;helper_bindings=[ordered]@{probe_runner=$ProbeRunnerSha256;probe=$ProbeSha256;frame_runner=$FrameRunnerSha256;capture=$CaptureHelperSha256};fixture_sha256=$null;created_process=$null;process_created=$null;initial_thread=$null;selected_window=$null;selected_thread=$null;suspended_before_job_assignment=$false;exact_image_verified_before_resume=$false;actual_private_job=$false;source_observation_sha256=$null;phone=$null;pixel_evidence=$null;editor_effect=$false;software_generated_input=$false;physical_pen_fidelity=$false;latency_acceptance=$false;input_settlement='NoInputStarted';editor_process_and_job_retired=$false;helper_processes_jobs_and_io_retired=$false;private_images_disposed=$false;failure_error_type=$null}
$receipt.failure_diagnostics=$diagnostic

function Get-PaintRefusalCode([string]$Message){
    # Exact static messages only. Unknown bodies never enter the receipt.
    switch -CaseSensitive ($Message){
        'Bounded Paint work deadline exceeded' {'work_deadline';break}
        'Native helper startup failed' {'helper_startup';break}
        'Native helper did not complete bounded work' {'helper_work';break}
        'Actual owned Paint foreground/root identity changed' {'editor_identity';break}
        'Actual mouse pointer must remain outside the owned Paint canvas' {'cursor_in_canvas';break}
        'Actual mouse cursor moved between canvas samples' {'cursor_between_samples';break}
        'Actual mouse cursor moved during canvas sample' {'cursor_during_sample';break}
        'Actual native WGC canvas sample refused' {'frame_verdict';break}
        'Exact native canvas filename required' {'canvas_filename';break}
        'Bound bytes changed' {'bound_bytes';break}
        'Stable native blank baseline required' {'baseline_stability';break}
        'Opaque sRGB baseline required' {'baseline_color';break}
        'Native canvas is not the fresh white fixture' {'baseline_white';break}
        'Actual stable Paint pixel effect refused' {'pixel_effect';break}
        'Private image disposal incomplete' {'private_disposal';break}
        default {'unknown'}
    }
}
function Save-PaintFrameDiagnostic([string]$Raw,[string]$ExpectedSample){
    # Best-effort closed projection only; never acceptance evidence.
    try{
        if($Raw.Length -gt 4194304){return}
        $v=$Raw.Trim().Split("`n")[-1]|ConvertFrom-Json -AsHashtable -Depth 55
        if($v.schema -ne 1 -or $v.operation -cne 'owned-blank-editor-wgc-metrics' -or $v.sampling_scope -cne 'verified_editor_canvas' -or $v.status -cnotin @('Measured','Failed','RetirementPending') -or $v.capture_session_id -cne $captureSession -or $v.sample_id -cne $ExpectedSample -or $v.helper_sha256 -cne $CaptureHelperSha256 -or -not $v.ContainsKey('error') -or -not $v.ContainsKey('capture_refusal')){return}
        $value=$v.error
        $diagnostic.native_error=if($null -eq $value){'none'}elseif($value -is [string] -and $value -cin @('Invalid','Limit','TargetChanged','Ungranted','Unavailable','Timeout','Cancelled','PartialInput','Io','RetirementPending')){$value}elseif($value -is [Collections.IDictionary] -and $value.Count -eq 1 -and @($value.Keys)[0] -ceq 'Platform'){'Platform'}else{'unknown'}
        $value=$v.capture_refusal
        $diagnostic.native_capture_refusal=if($null -eq $value){'none'}elseif($value -is [string] -and $value -cin @('invalid','consent','stale','own_window','unavailable','limit','color_depth','codec_missing','timeout','cancelled','busy','storage','platform','unsupported')){$value}else{'unknown'}
    }catch{} # Parsing cannot change the original guard or failure.
}

function Assert-Plain([string]$Path){
    $full=[IO.Path]::GetFullPath($Path)
    if($full -cnotmatch '\A[A-Za-z]:\\' -or $full.Substring(2).Contains(':') -or $full -cne $Path){throw 'Exact local path required'}
    $item=Get-Item -LiteralPath $full -Force
    while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected namespace refused'};$item=if($item -is [IO.DirectoryInfo]){$item.Parent}else{$item.Directory}}
}
function Pin-File([string]$Path,[string]$Sha256,[switch]$InstalledImage){
    if($Sha256 -cnotmatch '\A[a-f0-9]{64}\z'){throw 'Concrete SHA256 required'}
    # Package ancestors use the native installed-paint-v1 proof, which does not
    # claim ordinary namespace leases. Never alter WindowsApps permissions.
    if($InstalledImage){if([IO.Path]::GetFullPath($Path) -cne $Path -or (Get-Item -LiteralPath $Path).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Exact installed image required'}}else{Assert-Plain $Path}
    if(('M4FrameDirectory' -as [type]) -and -not $InstalledImage){
        $directory=(Get-Item -LiteralPath $Path).Directory
        while($null -ne $directory){if($directoryPins.Add($directory.FullName)){$pins.Add([M4FrameDirectory]::Open($directory.FullName))};$directory=$directory.Parent}
    }
    $file=[IO.File]::Open($Path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try{if([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($file)).ToLowerInvariant() -cne $Sha256){throw 'Bound bytes changed'};$file.Position=0;$pins.Add($file);return $file}catch{$file.Dispose();throw}
}
function Write-NewJson([string]$Path,$Value){
    $file=[IO.File]::Open($Path,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::Read)
    try{$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($Value|ConvertTo-Json -Depth 70 -Compress));$file.Write($bytes);$file.Flush($true)}finally{$file.Dispose()}
}
function Assert-WorkBudget {if($work.Elapsed.TotalSeconds -ge $WorkTimeoutSeconds){throw 'Bounded Paint work deadline exceeded'}}
function Wait-ChildSettlement($Child,[switch]$RequestRetirement){
    # Work has ended; this separate Pending loop has no destructive deadline.
    # A failed query never proves exit, Job emptiness or an actual IO join.
    $watch=[Diagnostics.Stopwatch]::StartNew();$next=0
    while($true){
        if($RequestRetirement){try{$Child.RequestRetirement()}catch{}}
        try{if($Child.IsSettled){return}}catch{}
        if($watch.Elapsed.TotalSeconds -ge $next){Write-Host 'Paint acceptance retains actual process/Job/IO owner while retirement is Pending';$next+=2}
        Start-Sleep -Milliseconds 50
    }
}
function Invoke-OwnedHelper([string]$Path,[string[]]$Arguments,[int]$Seconds=30,[switch]$Verdict,[ref]$VerdictExitCode,[ValidateSet('probe_runner','frame_runner','pixel_evaluator')][string]$Role,[string]$FrameSample){
    $diagnostic.helper_role=$Role;$diagnostic.helper_exit_code='unknown';$diagnostic.helper_timed_out='unknown';$diagnostic.helper_output_limit='unknown';$diagnostic.native_error='unknown';$diagnostic.native_capture_refusal='unknown'
    Assert-WorkBudget
    $child=[M4FrameChild]::Launch($Path,$Arguments,$true);$children.Add($child)
    $timer=[Diagnostics.Stopwatch]::StartNew();$timedOut=$false;$workCompleted=$false
    try{
        if($null -ne $child.StartupError){throw 'Native helper startup failed'}
        while(-not $child.IsSettled){
            if($timer.Elapsed.TotalSeconds -ge $Seconds -or $work.Elapsed.TotalSeconds -ge $WorkTimeoutSeconds){$timedOut=$true;break}
            Start-Sleep -Milliseconds 25
        }
        $workCompleted=-not $timedOut
    }finally{
        # These helpers are input-free. Their complete nested Job and output
        # workers must still settle before any private image can be removed.
        Wait-ChildSettlement $child -RequestRetirement:(-not $workCompleted)
        $diagnostic.helper_timed_out=$timedOut;$diagnostic.helper_output_limit=$child.OutputTooLarge
        try{$diagnostic.helper_exit_code=[uint32]$child.ExitCode}catch{}
    }
    $output=$child.StandardOutput
    $code=$child.ExitCode
    if($Role -ceq 'frame_runner' -and -not $child.OutputTooLarge){Save-PaintFrameDiagnostic $output $FrameSample}
    if($timedOut -or $child.OutputTooLarge -or ($code -ne 0 -and -not($Verdict -and $code -eq 1))){throw 'Native helper did not complete bounded work'}
    if($null -ne $VerdictExitCode){$VerdictExitCode.Value=$code}
    return $output
}
function Invoke-PaintProbe([string]$Mode,$Request){
    $json=$Request|ConvertTo-Json -Depth 15 -Compress
    $raw=Invoke-OwnedHelper $ProbeRunnerPath @($ProbePath,$ProbeSha256,$Mode,$json) -Role probe_runner
    $wrapper=$raw|ConvertFrom-Json -AsHashtable -Depth 50
    if($wrapper.schema -ne 1 -or $wrapper.status -cne 'Observed' -or $wrapper.probe_sha256 -cne $ProbeSha256 -or $wrapper.probe_exit_code -ne 0 -or $null -ne $wrapper.error -or $null -ne $wrapper.retirement_error -or $null -ne $wrapper.io_error -or $wrapper.suspended_before_job_assignment -ne $true -or $wrapper.exact_image_verified_before_resume -ne $true -or $wrapper.process_and_job_retired -ne $true -or $wrapper.io_joined -ne $true -or $wrapper.profile_authority -ne $false -or $wrapper.effect_harness -ne $false){throw 'Native probe owner proof refused'}
    $native=$wrapper.observation
    if($native.schema -ne 1 -or $native.parent_job_present -ne $true -or $native.source_namespace_policy -cne 'installed-paint-v1' -or $native.held_process_and_final_image -ne $true -or $native.held_ancestor_namespace -ne $false -or $native.namespace_lease_equivalence_claim -ne $false -or $native.os_package_namespace_validated -ne $true -or $native.foreground_same_selected_window_before -ne $true -or $native.foreground_same_selected_window_after -ne $true -or $native.input_sent -ne $false -or $native.profile_authority -ne $false){throw 'Native installed Paint witness refused'}
    return $native
}
function Assert-Editor {
    if($null -eq $editor -or -not $editor.IsOwnedJobMember($editor.Created) -or -not $editor.MatchesSelectedOwnedWindow($editor.Window,$editor.WindowThreadId,$editor.Created) -or -not $editor.IsForeground){throw 'Actual owned Paint foreground/root identity changed'}
}
function Read-CanvasCursor {
    $point=[M4FrameChild]::CursorPosition()
    if($point[0] -ge $canvas.x -and $point[0] -lt ([long]$canvas.x+$canvas.width) -and $point[1] -ge $canvas.y -and $point[1] -lt ([long]$canvas.y+$canvas.height)){throw 'Actual mouse pointer must remain outside the owned Paint canvas'}
    return [ordered]@{x=$point[0];y=$point[1]}
}
function Capture-Paint([ValidateSet('baseline1','baseline2','after1','after2')][string]$Label){
    $diagnostic.checkpoint=$Label+'_work_editor'
    Assert-WorkBudget;Assert-Editor
    $Sample=[M4FrameChild]::NewNativeId()
    $diagnostic.checkpoint=$Label+'_cursor_before';$cursorBefore=Read-CanvasCursor
    if($null -eq $script:cursorAnchor){$script:cursorAnchor=$cursorBefore}
    if($cursorBefore.x -ne $script:cursorAnchor.x -or $cursorBefore.y -ne $script:cursorAnchor.y){throw 'Actual mouse cursor moved between canvas samples'}
    $request=[ordered]@{schema=1;ownerPid=$PID;target=$target;image=$image;sourcePolicy='installed_paint';explicitOwnerCapture=$true;ownedBlankFixture=$true;captureHelperPath=$CaptureHelperPath;captureHelperSha256=$CaptureHelperSha256;captureSessionId=$captureSession;privateTempRoot=$privateRoot;markerNonce=$nativeRun;sampleId=$Sample;canvasRectHost=$canvas;samplingScope='verified_editor_canvas';expectedPaintFrame=$paintFrame}
    $diagnostic.checkpoint=$Label+'_helper'
    $raw=Invoke-OwnedHelper $FrameRunnerPath @(($request|ConvertTo-Json -Depth 20 -Compress)) -Role frame_runner -FrameSample $Sample
    $diagnostic.checkpoint=$Label+'_parse_verdict'
    $value=$raw.Trim().Split("`n")[-1]|ConvertFrom-Json -AsHashtable -Depth 55
    if($value.schema -ne 1 -or $value.status -cne 'Measured' -or $value.operation -cne 'owned-blank-editor-wgc-metrics' -or $value.sampling_scope -cne 'verified_editor_canvas' -or $value.capture_session_id -cne $captureSession -or $value.sample_id -cne $Sample -or $value.helper_sha256 -cne $CaptureHelperSha256 -or $value.helper_exit_code -ne 0 -or $value.process_and_job_retired -ne $true -or $value.io_worker_started -ne $true -or $value.io_join_completed -ne $true -or $value.suspended_before_job_assignment -ne $true -or $value.exact_image_verified_before_resume -ne $true -or $null -ne $value.error -or $null -ne $value.retirement_error -or $null -ne $value.io_error -or $null -ne $value.capture_refusal -or $value.input_sent -ne $false -or $value.profile_authority -ne $false -or $null -eq $value.canvas_metrics -or $null -eq $value.canvas_frame -or $null -ne $value.metrics -or $null -ne $value.frame){throw 'Actual native WGC canvas sample refused'}
    if($value.canvas_frame.filename -cne 'capture-canvas.png'){throw 'Exact native canvas filename required'}
    $png=Join-Path (Join-Path $privateRoot $Sample) $value.canvas_frame.filename
    $diagnostic.checkpoint=$Label+'_image_pin';[void](Pin-File $png $value.canvas_metrics.image.png_sha256)
    $canvasFiles.Add($png)
    $diagnostic.checkpoint=$Label+'_cursor_after';$cursorAfter=Read-CanvasCursor
    if($cursorAfter.x -ne $script:cursorAnchor.x -or $cursorAfter.y -ne $script:cursorAnchor.y){throw 'Actual mouse cursor moved during canvas sample'}
    $cursorSamples.Add([ordered]@{before=$cursorBefore;after=$cursorAfter})
    $diagnostic.checkpoint=$Label+'_editor_after';Assert-Editor
    return $value
}
function Wait-RemoteSettlement {
    if($null -eq $remoteSettlement -or $remoteSettlement.State -ceq 'NoInputStarted'){return}
    $clock=[Diagnostics.Stopwatch]::StartNew();$next=0
    while($true){
        if($remoteSettlement.State -ceq 'Retired'){return}
        try{
            $path=$remoteSettlement.NativeReceiptPath
            if($path -and [IO.Path]::GetDirectoryName($path) -ceq (Join-Path $repo '.local') -and [IO.Path]::GetFileName($path) -cmatch '\Aremote-normal-integration-[a-f0-9]{32}\.json\z'){
                Assert-Plain $path
                $file=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
                try{
                    if($file.Length -le 0 -or $file.Length -gt 262144){throw 'Bounded remote receipt required'}
                    $reader=[IO.StreamReader]::new($file);try{$value=$reader.ReadToEnd()|ConvertFrom-Json -AsHashtable -Depth 50}finally{$reader.Dispose()}
                    if($value.owned_editor_contract_sha256 -ceq $remoteSettlement.ContractSha256 -and (Test-M4InputRunnerRetired ([pscustomobject]$value) $value.run_id)){$remoteSettlement.State='Retired';return}
                }finally{$file.Dispose()}
            }
        }catch{}
        if($clock.Elapsed.TotalSeconds -ge $next){Write-Host 'Pending actual remote/input release; exact owned Paint document and Job remain alive';$next+=2}
        Start-Sleep -Milliseconds 50
    }
}

try{
    # Bootstrap compilation reads the manifest-bound native bytes under a lease.
    # All ordinary ancestor leases are then acquired before creating any child.
    [void](Pin-File $ArtifactManifest $ArtifactManifestSha256)
    $artifact=Get-Content -Raw -LiteralPath $ArtifactManifest|ConvertFrom-Json -AsHashtable -Depth 40
    if($artifact.schema -ne 1 -or $artifact.inventory_only -ne $true -or $artifact.root_build_review_required -ne $true){throw 'Root reviewed source/artifact inventory required'}
    $sourceHashes=[ordered]@{}
    foreach($source in $artifact.sources){if($source.path -cnotmatch '\A[A-Za-z0-9_./-]+\z' -or $source.path -match '(?:^|/)\.\.(?:/|$)' -or $sourceHashes.Contains($source.path)){throw 'Exact unique source cohort required'};$sourceHashes[$source.path]=$source.sha256}
    foreach($name in @('tools/remote-edit/paint_phone.ps1','tools/remote-edit/paint_phone_native.cs','tools/remote-edit/paint_phone_reports.py','tools/remote-edit/editor_phone_owner.ps1','tools/remote-edit/owned_editor_contract.ps1','tools/remote-edit/input_owner_contracts.ps1')){if(-not $sourceHashes.Contains($name)){throw 'Visible Paint caller source missing from artifact inventory'}}
    [void](Pin-File ([IO.Path]::GetFullPath((Join-Path $repo 'tools/remote-edit/paint_phone_native.cs'))) $sourceHashes['tools/remote-edit/paint_phone_native.cs'])
    if('M4FrameChild' -as [type]){throw 'Fresh PowerShell process required; preloaded owner type refused'}
    Add-Type -Path (Join-Path $PSScriptRoot 'paint_phone_native.cs')
    [M4FrameChild]::InitializeDpi()
    $nativeRun=[M4FrameChild]::NewNativeId()
    [void](Pin-File $ArtifactManifest $ArtifactManifestSha256)
    foreach($source in $artifact.sources){[void](Pin-File ([IO.Path]::GetFullPath((Join-Path $repo $source.path))) $source.sha256)}
    $receipt.source_count=$sourceHashes.Count
    foreach($entry in @($artifact.source_map)+@($artifact.build_receipts)+@($artifact.classpath_bindings)+@($artifact.classpath,$artifact.java,$artifact.harness,$artifact.apk.main,$artifact.apk.test)){[void](Pin-File $entry.path $entry.sha256)}
    foreach($name in $artifact.native.Keys){[void](Pin-File ([IO.Path]::GetFullPath((Join-Path $repo ('target/debug/'+$name)))) $artifact.native[$name])}
    foreach($binding in @(@($ProbeRunnerPath,$ProbeRunnerSha256,'vw-editor-probe-runner.exe'),@($ProbePath,$ProbeSha256,'vw-editor-probe.exe'),@($FrameRunnerPath,$FrameRunnerSha256,'vw-editor-frame-runner.exe'),@($CaptureHelperPath,$CaptureHelperSha256,'vw-capture-helper.exe'))){if([IO.Path]::GetFileName($binding[0]) -cne $binding[2]){throw 'Exact native helper filename required'};[void](Pin-File $binding[0] $binding[1])}
    if($artifact.native['vw-capture-helper.exe'] -cne $CaptureHelperSha256){throw 'Capture helper differs from admitted remote cohort'}
    if([IO.Path]::GetFileName($PaintPath) -cne 'mspaint.exe'){throw 'Exact installed Paint image required'}
    [void](Pin-File $PaintPath $PaintSha256 -InstalledImage)
    $pythonCommand=Get-Command $PythonPath -CommandType Application
    $PythonPath=$pythonCommand.Source
    $pythonSha=(Get-FileHash -LiteralPath $PythonPath -Algorithm SHA256).Hash.ToLowerInvariant()
    [void](Pin-File $PythonPath $pythonSha)
    . (Join-Path $PSScriptRoot 'editor_phone_owner.ps1')
    . (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
    Assert-WorkBudget
    $phase='owned_blank_launch';$diagnostic.checkpoint=$phase
    $privateRoot=Join-Path ([IO.Path]::GetTempPath()) ('VisualWorkbench-editor-metrics-'+$nativeRun)
    $fixtureRoot=Join-Path ([IO.Path]::GetTempPath()) ('VisualWorkbench-paint-phone-'+$run)
    foreach($directory in @($privateRoot,$fixtureRoot)){[M4FrameChild]::CreateOwnedDirectory($directory);$ownedDirectories.Add($directory);Assert-Plain $directory}
    Write-NewJson (Join-Path $privateRoot '.vw-editor-metrics-owner') ([ordered]@{schema=1;ownerPid=$PID;nonce=$nativeRun})
    $fixtureStem='blank-'+$run;$fixture=Join-Path $fixtureRoot ($fixtureStem+'.bmp')
    [M4FrameChild]::CreateBlankFixture($fixture)
    $receipt.fixture_sha256=(Get-FileHash -LiteralPath $fixture -Algorithm SHA256).Hash.ToLowerInvariant()
    [void](Pin-File $fixture $receipt.fixture_sha256)
    $editor=[M4FrameChild]::Launch($PaintPath,@($fixture),$false)
    $receipt.created_process=$editor.ProcessId;$receipt.process_created=[string]$editor.Created;$receipt.initial_thread=$editor.InitialThreadId
    $receipt.suspended_before_job_assignment=$editor.SuspendedBeforeAssignment;$receipt.exact_image_verified_before_resume=$editor.ExactImageVerifiedBeforeResume;$receipt.actual_private_job=$editor.IsOwnedJobMember($editor.Created)
    if($null -ne $editor.StartupError){throw 'Exact fresh Paint startup failed'}
    $ready=[Diagnostics.Stopwatch]::StartNew()
    while(-not $editor.TrySelectWindow($fixtureStem)){Assert-WorkBudget;if($editor.HasExited -or $ready.Elapsed.TotalSeconds -ge 15){throw 'Created Paint did not own the fixture root; delegated/existing windows refused'};Start-Sleep -Milliseconds 50}
    $receipt.selected_window=$editor.Window;$receipt.selected_thread=$editor.WindowThreadId
    $focus=[Diagnostics.Stopwatch]::StartNew()
    while(-not $editor.IsForeground){Assert-WorkBudget;if($focus.Elapsed.TotalSeconds -ge 5){throw 'Normal owned Paint foreground acquisition refused'};$editor.RequestNormalForeground();Start-Sleep -Milliseconds 100}
    Assert-Editor
    $phase='native_canvas_binding';$diagnostic.checkpoint=$phase
    $bound=Invoke-PaintProbe '--bind-paint' ([ordered]@{schema=1;window=$editor.Window;process_id=$editor.Process.Id;owner_pid=$PID;token=$nativeRun})
    $target=$bound.target;$image=$bound.image
    if([string]$target.process_created -cne [string]$editor.Created -or $target.process_id -ne $editor.Process.Id -or $target.window -ne $editor.Window -or $target.thread_id -ne $editor.WindowThreadId -or $image.executableName -cne 'mspaint.exe'){throw 'Native binding differs from created Paint identity'}
    $observed=Invoke-PaintProbe '--observe-paint-controls' ([ordered]@{schema=1;owner_pid=$PID;target=$target;image=$image})
    $controls=$observed.observation.controls
    if(($observed.target|ConvertTo-Json -Depth 15 -Compress) -cne ($target|ConvertTo-Json -Depth 15 -Compress) -or ($observed.image|ConvertTo-Json -Depth 15 -Compress) -cne ($image|ConvertTo-Json -Depth 15 -Compress) -or $controls.requiredSettingsKnown -ne $true -or $controls.selectedRootOnly -ne $true -or $controls.inputSent -ne $false -or $controls.profileAuthority -ne $false -or $controls.effectProven -ne $false -or $controls.recognizedTool.status -cne 'known' -or $controls.recognizedTool.value -cne 'brush' -or $controls.frameWitness.status -cne 'known'){throw 'Native Paint brush/canvas controls unproved'}
    $paintFrame=$controls.frameWitness.value;$canvas=$paintFrame.canvasRectHost
    $observationPath=Join-Path $fixtureRoot 'source-observation.json';Write-NewJson $observationPath $observed
    $receipt.source_observation_sha256=(Get-FileHash -LiteralPath $observationPath -Algorithm SHA256).Hash.ToLowerInvariant()
    [void](Pin-File $observationPath $receipt.source_observation_sha256)
    $captureSession=[M4FrameChild]::NewNativeId()
    $phase='baseline_wgc';$baseline=@((Capture-Paint 'baseline1'),(Capture-Paint 'baseline2'))
    # Refuse a nonblank or unstable baseline BEFORE authorizing phone input.
    $first=$baseline[0].canvas_metrics.image;$second=$baseline[1].canvas_metrics.image
    $diagnostic.checkpoint='baseline_assertions'
    if($first.decoded_rgba_sha256 -cne $second.decoded_rgba_sha256 -or $first.canvas_pixels -ne ([long]$canvas.width*$canvas.height)){throw 'Stable native blank baseline required'}
    foreach($sample in $baseline){$metric=$sample.canvas_metrics.image;if($metric.alpha_not_opaque_pixels -ne 0 -or $metric.explicit_srgb -ne $true){throw 'Opaque sRGB baseline required'};foreach($channel in 0..3){if($metric.rgba_min[$channel] -ne 255 -or $metric.rgba_max[$channel] -ne 255 -or $metric.rgba_sum[$channel] -ne ([long]$metric.canvas_pixels*255)){throw 'Native canvas is not the fresh white fixture'}}}
    Assert-WorkBudget;Assert-Editor
    $phase='phone_stroke';$diagnostic.checkpoint=$phase
    Write-Host 'Dispatching synthetic short S23 phone stroke into the exact owned Paint canvas'
    $phoneStarted=[M4FrameChild]::Clock100ns()
    $phone=Invoke-M4OwnedEditorPhoneStroke -OwnedEditor $editor -EditorKind paint -Target $target -CanvasRect $canvas -ExecutableSha256 $PaintSha256 -SourceObservationSha256 $receipt.source_observation_sha256 -PrivateRunDirectory $fixtureRoot -ArtifactManifest $ArtifactManifest -ArtifactManifestSha256 $ArtifactManifestSha256 -SourceHashes $sourceHashes -UsbInterfaceIndex $UsbInterfaceIndex -UsbDeviceInstanceSha256 $UsbDeviceInstanceSha256 -PcUsbAddress $PcUsbAddress -PhoneUsbAddress $PhoneUsbAddress -Port $Port -Settlement ([ref]$remoteSettlement)
    $phone.started_qpc_100ns=$phoneStarted;$phone.completed_qpc_100ns=[M4FrameChild]::Clock100ns()
    Wait-RemoteSettlement
    $receipt.input_settlement=$remoteSettlement.State;$receipt.phone=$phone;$receipt.software_generated_input=$true
    # Remote execution has its own bounded work and retained Pending owner. The
    # local post-sampling budget begins only after actual release, never while
    # it still owes a pointer/key/contact release to this destination.
    $work.Restart();$phase='after_wgc';$after=@((Capture-Paint 'after1'),(Capture-Paint 'after2'))
    [void](Pin-File $remoteSettlement.NativeReceiptPath $phone.native_receipt_sha256)
    $nativePhone=Get-Content -Raw -LiteralPath $remoteSettlement.NativeReceiptPath|ConvertFrom-Json -AsHashtable -Depth 55
    $evaluation=[ordered]@{schema=1;target=$target;image=$image;paint_frame=$paintFrame;canvas_rect=$canvas;baseline=$baseline;after=$after;phone=$phone;native_phone=$nativePhone;canvas_files=@($canvasFiles);stroke_ppm=$phone.stroke_ppm;stroke_radius_pixels=16;cursor_samples=@($cursorSamples)}
    $evaluationPath=Join-Path $fixtureRoot 'pixel-evaluation.json';Write-NewJson $evaluationPath $evaluation
    [void](Pin-File $evaluationPath (Get-FileHash -LiteralPath $evaluationPath -Algorithm SHA256).Hash.ToLowerInvariant())
    $phase='pixel_assertion';$diagnostic.checkpoint=$phase
    $pixelExit=$null
    $evaluated=Invoke-OwnedHelper $PythonPath @((Join-Path $PSScriptRoot 'paint_phone_reports.py'),'--input',$evaluationPath) -Verdict -VerdictExitCode ([ref]$pixelExit) -Role pixel_evaluator
    $proof=$evaluated|ConvertFrom-Json -AsHashtable -Depth 30
    $receipt.pixel_evidence=$proof
    if($pixelExit -ne 0 -or $proof.schema -ne 1 -or $proof.status -cne 'passed' -or $proof.accepted -ne $true -or $proof.editor_effect -ne $true){throw 'Actual stable Paint pixel effect refused'}
    $receipt.editor_effect=$true;$receipt.status='passed';$receipt.failure_phase=$null
    if($InspectSeconds -gt 0){Write-Host ('Verified Paint canvas pixel change; retaining visible owned document for '+$InspectSeconds+' seconds');$inspection=[Diagnostics.Stopwatch]::StartNew();while($inspection.Elapsed.TotalSeconds -lt $InspectSeconds -and -not $editor.HasExited){Start-Sleep -Milliseconds 100}}
}catch{
    $receipt.failure_phase=$phase;$receipt.failure_error_type=$_.Exception.GetType().Name;$diagnostic.refusal_code=Get-PaintRefusalCode $_.Exception.Message
    Write-Host ('Paint phone visible-drawing acceptance failed in phase='+$phase+'; private details withheld')
}finally{
    # Never finally-kill an editor while the direct adapter owes real release.
    Wait-RemoteSettlement
    if($null -ne $remoteSettlement){$receipt.input_settlement=$remoteSettlement.State}
    foreach($child in $children){Wait-ChildSettlement $child -RequestRetirement;try{[void]$child.StandardOutput}catch{};$child.Dispose()}
    $receipt.helper_processes_jobs_and_io_retired=$true
    if($null -ne $editor){Wait-ChildSettlement $editor -RequestRetirement;$receipt.editor_process_and_job_retired=$true;$editor.Dispose()}
    # Drop deny-delete leases only after all input, capture and IO owners retire.
    for($i=$pins.Count-1;$i -ge 0;$i--){$pins[$i].Dispose()}
    try{
        foreach($directory in $ownedDirectories){
            Assert-Plain $directory
            $entries=@(Get-ChildItem -LiteralPath $directory -Recurse -Force)
            foreach($entry in $entries){Assert-Plain $entry.FullName}
            # These fresh private roots contain only this run's fixture, source
            # witness, evaluator input, and native WGC samples. No repo captures.
            Remove-Item -LiteralPath $directory -Recurse -Force
            if(Test-Path -LiteralPath $directory){throw 'Private image disposal incomplete'}
        }
        $receipt.private_images_disposed=$true
    }catch{$receipt.status='failed';$receipt.failure_phase='private_disposal';$receipt.failure_error_type=$_.Exception.GetType().Name;$diagnostic.checkpoint='private_disposal';$diagnostic.refusal_code=Get-PaintRefusalCode $_.Exception.Message}
    $local=Join-Path $repo '.local';if(-not(Test-Path -LiteralPath $local)){[void][IO.Directory]::CreateDirectory($local)};Assert-Plain $local
    Write-NewJson (Join-Path $local ('paint-phone-visible-'+$run+'.json')) $receipt
}
Write-Host ('Paint phone visible-drawing receipt: paint-phone-visible-'+$run+'.json; status='+$receipt.status)
if($receipt.status -cne 'passed'){exit 1}

# Called directly by the retained editor parent, after its native observation
# and baseline WGC sample. It does not create, close, focus or dispose an editor.
Set-StrictMode -Version Latest
function Invoke-M4OwnedEditorPhoneStroke {
    [CmdletBinding()]
    param(
        [Parameter(Mandatory)][object]$OwnedEditor,
        [Parameter(Mandatory)][ValidateSet('krita','paint')][string]$EditorKind,
        [Parameter(Mandatory)][Collections.IDictionary]$Target,
        [Parameter(Mandatory)][Collections.IDictionary]$CanvasRect,
        [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$ExecutableSha256,
        [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$SourceObservationSha256,
        [Parameter(Mandatory)][string]$PrivateRunDirectory,
        [Parameter(Mandatory)][string]$ArtifactManifest,
        [Parameter(Mandatory)][ValidatePattern('\A[a-f0-9]{64}\z')][string]$ArtifactManifestSha256,
        [Parameter(Mandatory)][Collections.IDictionary]$SourceHashes,
        [Parameter(Mandatory)][int]$UsbInterfaceIndex,
        [Parameter(Mandatory)][string]$UsbDeviceInstanceSha256,
        [Parameter(Mandatory)][string]$PcUsbAddress,
        [Parameter(Mandatory)][string]$PhoneUsbAddress,
        [Parameter(Mandatory)][int]$Port,
        [ref]$Settlement
    )
    $repo=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
    $pins=[Collections.Generic.List[IO.FileStream]]::new()
    $native=$null;$returned=$false;$ownerExit=$null
    # Only this source-owned boundary reports whether input was ever admitted.
    # A caller may retain the same mutable state object through every exception.
    $settlementState=[ordered]@{State='NoInputStarted';NativeReceiptPath=$null;ContractSha256=$null}
    if($null -ne $Settlement){$Settlement.Value=$settlementState}
    try{
        # The caller retains the complete current source/artifact cohort. These
        # executable owner dependencies are additionally leased across the call.
        foreach($name in @('tools/remote-edit/integration_input_owner.ps1','tools/remote-edit/owned_editor_contract.ps1','tools/remote-edit/input_owner_contracts.ps1','tools/remote-edit/input_owner_startup.ps1','tools/remote-edit/input_process_output.cs','tools/process.psm1')){
            if(-not $SourceHashes.Contains($name) -or $SourceHashes[$name] -cnotmatch '\A[a-f0-9]{64}\z'){throw 'Bound editor phone dependency required'}
            $path=Join-Path $repo $name;$item=Get-Item -LiteralPath $path
            while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected editor phone dependency'};$item=if($item -is [IO.DirectoryInfo]){$item.Parent}else{$item.Directory}}
            $stream=[IO.File]::Open($path,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
            try{if([Convert]::ToHexString([Security.Cryptography.SHA256]::HashData($stream)).ToLowerInvariant() -cne $SourceHashes[$name]){throw 'Editor phone dependency changed'};$stream.Position=0;$pins.Add($stream)}catch{$stream.Dispose();throw}
        }
        . (Join-Path $PSScriptRoot 'owned_editor_contract.ps1')
        . (Join-Path $PSScriptRoot 'input_owner_contracts.ps1')
        if($OwnedEditor.GetType().FullName -cne 'M4FrameChild' -or $OwnedEditor.Process.Id -ne $Target.process_id -or -not $OwnedEditor.IsOwnedJobMember([uint64]$Target.process_created) -or -not $OwnedEditor.MatchesSelectedOwnedWindow([long]$Target.window,[uint32]$Target.thread_id,[uint64]$Target.process_created)){throw 'Actual retained editor ownership required'}
        $self=[Diagnostics.Process]::GetCurrentProcess()
        try{
            # Resolve three short interior points from the contemporaneous native
            # canvas/client proof. The native guard rechecks the actual target at
            # grant and each input; geometry here grants no authority.
            $client=$Target.client_rect
            $x=[int][Math]::Round((([double]$CanvasRect.x+$CanvasRect.width/2.0-$client.x)/$client.width)*1000000)
            $y=[int][Math]::Round((([double]$CanvasRect.y+$CanvasRect.height/2.0-$client.y)/$client.height)*1000000)
            $contract=[ordered]@{schema='M4_REMOTE_OWNED_EDITOR_V1';parent_pid=$PID;parent_created=[string]$self.StartTime.ToUniversalTime().ToFileTimeUtc();editor_kind=$EditorKind;editor_pid=[int]$Target.process_id;editor_created=[string]$Target.process_created;editor_thread=[long]$Target.thread_id;window=[long]$Target.window;executable_name=$(if($EditorKind -ceq 'krita'){'krita.exe'}else{'mspaint.exe'});executable_sha256=$ExecutableSha256;nonce=[Guid]::NewGuid().ToString('N');job_assigned=$true;blank_fixture=$true;canvas_rect=$CanvasRect;client_rect=$client;stroke_ppm=@($x,$y,($x+1000),$y,($x+2000),$y);source_observation_sha256=$SourceObservationSha256}
            Assert-M4OwnedEditorContract $contract $PID $contract.parent_created
        }finally{$self.Dispose()}
        Assert-M4OwnedEditorAlive $contract
        Assert-M4OwnedEditorOwner $OwnedEditor $contract
        $item=Get-Item -LiteralPath $PrivateRunDirectory
        if($item -isnot [IO.DirectoryInfo]){throw 'Existing private editor directory required'}
        while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected editor private directory'};$item=$item.Parent}
        $contractPath=Join-Path $PrivateRunDirectory ('owned-editor-'+$contract.nonce+'.json')
        $file=[IO.File]::Open($contractPath,[IO.FileMode]::CreateNew,[IO.FileAccess]::Write,[IO.FileShare]::None)
        try{$bytes=[Text.UTF8Encoding]::new($false).GetBytes(($contract|ConvertTo-Json -Depth 8 -Compress));$file.Write($bytes);$file.Flush($true)}finally{$file.Dispose()}
        $contractHash=(Get-FileHash -LiteralPath $contractPath -Algorithm SHA256).Hash.ToLowerInvariant()
        $settlementState.ContractSha256=$contractHash
        # Direct invocation is essential. A timeout wrapper must not terminate
        # this parent or destination while the inner release remains Pending.
        $nativePath=$null
        $settlementState.State='Pending'
        & (Join-Path $PSScriptRoot 'integration_input_owner.ps1') -Execute -OwnerReady -ArtifactManifest $ArtifactManifest -ArtifactManifestSha256 $ArtifactManifestSha256 -UsbInterfaceIndex $UsbInterfaceIndex -UsbDeviceInstanceSha256 $UsbDeviceInstanceSha256 -PcUsbAddress $PcUsbAddress -PhoneUsbAddress $PhoneUsbAddress -Port $Port -InputScenario balanced -OwnedEditorContract $contractPath -OwnedEditorContractSha256 $contractHash -OwnedEditorOwner $OwnedEditor -CompletedReceiptPath ([ref]$nativePath)
        $ownerExit=$LASTEXITCODE;$returned=$true
        # The retained entry point returns its exact fresh receipt only after
        # actual settlement. Presence/exit alone is never input acceptance.
        if($null -eq $nativePath -or [IO.Path]::GetDirectoryName($nativePath) -cne (Join-Path $repo '.local') -or [IO.Path]::GetFileName($nativePath) -cnotmatch '\Aremote-normal-integration-[a-f0-9]{32}\.json\z'){throw 'Exact returned editor phone receipt required'}
        $settlementState.NativeReceiptPath=$nativePath
        $item=Get-Item -LiteralPath $nativePath
        if($item.Length -le 0 -or $item.Length -gt 262144){throw 'Editor phone receipt bound'}
        while($null -ne $item){if(($item.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0){throw 'Redirected editor phone receipt'};$item=if($item -is [IO.DirectoryInfo]){$item.Parent}else{$item.Directory}}
        $nativeLease=[IO.File]::Open($nativePath,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read);$pins.Add($nativeLease)
        $native=Get-Content -LiteralPath $nativePath -Raw|ConvertFrom-Json -AsHashtable -Depth 40
        if($native.owned_editor_contract_sha256 -cne $contractHash){throw 'Editor phone contract mismatch'}
        if($null -eq $native -or -not(Test-M4InputRunnerRetired ([pscustomobject]$native) $native.run_id)){throw 'Actual editor phone/native/input/Job/IO settlement missing'}
        # Retirement is independent of the later phone/pixel acceptance verdict.
        $settlementState.State='Retired'
        if($ownerExit -ne 0 -or $native.status -cne 'passed' -or $native.editor_phone_records_validated -ne $true -or $native.phone_render_records_validated -ne $true -or $native.external_editor_destination_retained -ne $true -or $native.controller_input_records_validated -ne $false -or $native.editor_effect -ne $false -or $native.owned_packages_cleanup -ne $true){throw 'Owned editor phone correlation failed'}
        if(-not $OwnedEditor.IsOwnedJobMember([uint64]$Target.process_created) -or -not $OwnedEditor.MatchesSelectedOwnedWindow([long]$Target.window,[uint32]$Target.thread_id,[uint64]$Target.process_created)){throw 'Editor ownership changed after remote retirement'}
        return [ordered]@{schema=1;status='phone-stroke-correlated';native_receipt_path=$nativePath;native_receipt_sha256=(Get-FileHash -LiteralPath $nativePath -Algorithm SHA256).Hash.ToLowerInvariant();contract_sha256=$contractHash;stroke_ppm=$contract.stroke_ppm;actual_remote_retirement=$true;retained_parent_returned=$returned;receiver_observed=$false;editor_effect=$false;local_history_may_begin=$true;physical_pen_fidelity=$false;latency_acceptance=$false}
    }finally{
        # Never dispose, close or kill $OwnedEditor. The caller still owns its
        # Job and document, including any failure before or after this adapter.
        foreach($pin in $pins){$pin.Dispose()}
    }
}

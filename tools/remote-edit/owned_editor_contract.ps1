# Closed geometry/identity data for an editor retained by the assigned parent.
# This contract never grants input. The normal native grant and per-batch guards
# still independently admit the exact selected HWND/PID/creation identity.
Set-StrictMode -Version Latest
function Test-M4EditorInteger($Value,[long]$Minimum,[long]$Maximum){
    return ($Value -is [int] -or $Value -is [long]) -and $Value -ge $Minimum -and $Value -le $Maximum
}
function Assert-M4EditorKeys($Value,[string[]]$Expected){
    if($Value -isnot [Collections.IDictionary] -or ((@($Value.Keys|Sort-Object)-join ',') -cne (@($Expected|Sort-Object)-join ','))){throw 'Owned editor contract shape refused'}
}
function Assert-M4EditorRect($Value){
    Assert-M4EditorKeys $Value @('x','y','width','height')
    foreach($key in @('x','y')){if(-not(Test-M4EditorInteger $Value[$key] -2147483648 2147483647)){throw 'Owned editor geometry refused'}}
    foreach($key in @('width','height')){if(-not(Test-M4EditorInteger $Value[$key] 32 32768)){throw 'Owned editor geometry refused'}}
}
function Assert-M4OwnedEditorContract($Value,[int]$ParentId,[string]$ParentCreated){
    Assert-M4EditorKeys $Value @('schema','parent_pid','parent_created','editor_kind','editor_pid','editor_created','editor_thread','window','executable_name','executable_sha256','nonce','job_assigned','blank_fixture','canvas_rect','client_rect','stroke_ppm','source_observation_sha256')
    if($Value.schema -cne 'M4_REMOTE_OWNED_EDITOR_V1' -or -not(Test-M4EditorInteger $Value.parent_pid 1 2147483647) -or $Value.parent_pid -ne $ParentId -or $Value.parent_created -isnot [string] -or $Value.parent_created -cne $ParentCreated){throw 'Owned editor parent binding refused'}
    if($Value.editor_kind -cnotin @('krita','paint') -or $Value.executable_name -cne $(if($Value.editor_kind -ceq 'krita'){'krita.exe'}else{'mspaint.exe'})){throw 'Owned editor kind refused'}
    if(-not(Test-M4EditorInteger $Value.editor_pid 1 2147483647) -or $Value.editor_pid -eq $ParentId -or $Value.editor_created -isnot [string] -or $Value.editor_created -cnotmatch '\A[1-9][0-9]{0,18}\z' -or -not(Test-M4EditorInteger $Value.window 1 ([long]::MaxValue))){throw 'Owned editor process identity refused'}
    if(-not(Test-M4EditorInteger $Value.editor_thread 1 4294967295)){throw 'Owned editor thread refused'}
    foreach($key in @('executable_sha256','source_observation_sha256')){if($Value[$key] -isnot [string] -or $Value[$key] -cnotmatch '\A[a-f0-9]{64}\z'){throw 'Owned editor observation binding refused'}}
    if($Value.nonce -isnot [string] -or $Value.nonce -cnotmatch '\A[a-f0-9]{32}\z' -or $Value.job_assigned -isnot [bool] -or $Value.job_assigned -ne $true -or $Value.blank_fixture -isnot [bool] -or $Value.blank_fixture -ne $true){throw 'Owned blank editor assignment required'}
    Assert-M4EditorRect $Value.canvas_rect;Assert-M4EditorRect $Value.client_rect
    $canvas=$Value.canvas_rect;$client=$Value.client_rect
    if($canvas.x -lt $client.x -or $canvas.y -lt $client.y -or ([long]$canvas.x+$canvas.width) -gt ([long]$client.x+$client.width) -or ([long]$canvas.y+$canvas.height) -gt ([long]$client.y+$client.height)){throw 'Observed canvas outside client'}
    if($Value.stroke_ppm -isnot [array] -or $Value.stroke_ppm.Count -ne 6){throw 'Exact three editor points required'}
    foreach($coordinate in $Value.stroke_ppm){if(-not(Test-M4EditorInteger $coordinate 1000 999000)){throw 'Editor stroke coordinate refused'}}
    if($Value.stroke_ppm[2]-$Value.stroke_ppm[0] -ne 1000 -or $Value.stroke_ppm[4]-$Value.stroke_ppm[0] -ne 2000 -or $Value.stroke_ppm[1] -ne $Value.stroke_ppm[3] -or $Value.stroke_ppm[1] -ne $Value.stroke_ppm[5]){throw 'Exact short editor stroke required'}
    for($i=0;$i -lt 3;$i++){
        $x=[double]$client.x+[double]$Value.stroke_ppm[2*$i]*$client.width/1000000.0
        $y=[double]$client.y+[double]$Value.stroke_ppm[2*$i+1]*$client.height/1000000.0
        if($x -lt ($canvas.x+4) -or $y -lt ($canvas.y+4) -or $x -gt ([long]$canvas.x+$canvas.width-4) -or $y -gt ([long]$canvas.y+$canvas.height-4)){throw 'Editor stroke leaves observed canvas interior'}
    }
}
function Assert-M4OwnedEditorAlive($Value){
    $parent=[Diagnostics.Process]::GetProcessById([int]$Value.parent_pid)
    $editor=[Diagnostics.Process]::GetProcessById([int]$Value.editor_pid)
    try{
        if($parent.HasExited -or $editor.HasExited -or [string]$parent.StartTime.ToUniversalTime().ToFileTimeUtc() -cne $Value.parent_created -or [string]$editor.StartTime.ToUniversalTime().ToFileTimeUtc() -cne $Value.editor_created){throw 'Owned editor live identity changed'}
        $image=$editor.MainModule.FileName
        if([IO.Path]::GetFileName($image) -cne $Value.executable_name -or (Get-FileHash -LiteralPath $image -Algorithm SHA256).Hash.ToLowerInvariant() -cne $Value.executable_sha256){throw 'Owned editor loaded image binding changed'}
    }finally{$editor.Dispose();$parent.Dispose()}
}
function Assert-M4OwnedEditorOwner($Owner,$Value){
    if($null -eq $Owner -or $Owner.GetType().FullName -cne 'M4FrameChild' -or $Owner.Process.Id -ne $Value.editor_pid -or -not $Owner.IsOwnedJobMember([uint64]$Value.editor_created) -or -not $Owner.MatchesSelectedOwnedWindow([long]$Value.window,[uint32]$Value.editor_thread,[uint64]$Value.editor_created)){throw 'Actual retained editor Job/root proof failed'}
}

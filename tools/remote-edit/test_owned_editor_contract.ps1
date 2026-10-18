Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
. (Join-Path $PSScriptRoot 'owned_editor_contract.ps1')
function Fixture {
    return @{schema='M4_REMOTE_OWNED_EDITOR_V1';parent_pid=31;parent_created='134000000000000001';editor_kind='krita';editor_pid=41;editor_created='134000000000000002';editor_thread=42;window=101;executable_name='krita.exe';executable_sha256=('a'*64);nonce=('b'*32);job_assigned=$true;blank_fixture=$true;canvas_rect=@{x=100;y=100;width=800;height=600};client_rect=@{x=0;y=0;width=1000;height=800};stroke_ppm=@(500000,500000,501000,500000,502000,500000);source_observation_sha256=('c'*64)}
}
$count=0
foreach($kind in @('krita','paint')){
    $f=Fixture;$f.editor_kind=$kind;$f.executable_name=if($kind -ceq 'krita'){'krita.exe'}else{'mspaint.exe'}
    Assert-M4OwnedEditorContract $f 31 '134000000000000001';$count++
}
$mutations=@(
    {$args[0].parent_pid=32},{$args[0].parent_created='134000000000000003'},
    {$args[0].editor_pid=31},{$args[0].editor_created=12},{$args[0].editor_thread=0},
    {$args[0].window=0},{$args[0].window=1.5},{$args[0].editor_kind='other'},
    {$args[0].executable_name='mspaint.exe'},{$args[0].executable_sha256='bad'},
    {$args[0].job_assigned=$false},{$args[0].job_assigned='true'},
    {$args[0].blank_fixture=$false},{$args[0].nonce='bad'},
    {$args[0].canvas_rect.x=-1},{$args[0].canvas_rect.width=1001},
    {$args[0].stroke_ppm[0]=500000.0},{$args[0].stroke_ppm[2]=502000},
    {$args[0].stroke_ppm[5]=501000},{$args[0].stroke_ppm=@(1000,1000,2000,1000,3000,1000)},
    {$args[0].stroke_ppm=@(1,2)},{$args[0].source_observation_sha256='bad'},
    {$args[0].unexpected=$true},{$args[0].Remove('job_assigned')}
)
foreach($mutate in $mutations){
    $f=Fixture;& $mutate $f|Out-Null;$refused=$false
    try{Assert-M4OwnedEditorContract $f 31 '134000000000000001'}catch{$refused=$true}
    if(-not $refused){throw 'Invalid retained editor contract admitted'};$count++
}
Write-Output ('Owned editor pure contract cases passed='+$count)
# Execute the actual owner admission helper against a pure retained-owner seam.
# No native API, process start, editor or UI is exercised by these cases.
Add-Type -TypeDefinition @'
public sealed class M4FrameChild {
 public sealed class Identity { public int Id=41; }
 public Identity Process=new Identity();
 public bool Job=true,Root=true;
 public int JobChecks=0,RootChecks=0;
 public bool IsOwnedJobMember(ulong created){JobChecks++;return Job&&created==134000000000000002UL;}
 public bool MatchesSelectedOwnedWindow(long hwnd,uint thread,ulong created){RootChecks++;return Root&&hwnd==101&&thread==42&&created==134000000000000002UL;}
}
'@
$f=Fixture;$owner=[M4FrameChild]::new();Assert-M4OwnedEditorOwner $owner $f
if($owner.JobChecks -ne 1 -or $owner.RootChecks -ne 1){throw 'Actual owner query census'}
foreach($kind in @('missing','foreign-type','wrong-pid','not-in-job','wrong-root')){
    $owner=[M4FrameChild]::new()
    switch($kind){'missing'{$owner=$null};'foreign-type'{$owner=[pscustomobject]@{Assigned=$true;Process=@{Id=41}}};'wrong-pid'{$owner.Process.Id=99};'not-in-job'{$owner.Job=$false};'wrong-root'{$owner.Root=$false}}
    $refused=$false;try{Assert-M4OwnedEditorOwner $owner $f}catch{$refused=$true}
    if(-not $refused){throw 'Unproven actual editor Job/root admitted'}
}
Write-Output 'Owned editor actual-helper owner seam cases passed=6'

#requires -Version 7.2
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
# Extract only pure diagnostic functions; never execute the acceptance parent.
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'paint_phone.ps1'),[ref]$tokens,[ref]$errors)
if($errors.Count -ne 0){throw 'Paint parent syntax errors'}
foreach($name in @('Get-PaintRefusalCode','Save-PaintFrameDiagnostic')){
    $found=@($ast.FindAll({param($n)$n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq $name},$true))
    if($found.Count -ne 1){throw 'Exact diagnostic function missing'}
    . ([scriptblock]::Create($found[0].Extent.Text))
}
$captureSession='019f7c21-5678-7000-8000-000000000001';$sample='019f7c21-5678-7000-8000-000000000002';$CaptureHelperSha256='a'*64
$script:cases=0
function Assert($Value){if(-not $Value){throw 'Closed diagnostic fixture failed'};$script:cases++}
function New-Receipt {return [ordered]@{schema=1;operation='owned-blank-editor-wgc-metrics';sampling_scope='verified_editor_canvas';status='Failed';capture_session_id=$captureSession;sample_id=$sample;helper_sha256=$CaptureHelperSha256;error='Unavailable';capture_refusal='color_depth';private_payload='private/path and coordinates must not survive'}}
function Project($Value){
    $script:diagnostic=[ordered]@{native_error='unknown';native_capture_refusal='unknown'}
    Save-PaintFrameDiagnostic ($Value|ConvertTo-Json -Depth 10 -Compress) $sample
    Assert ($diagnostic.Count -eq 2 -and ($diagnostic|ConvertTo-Json -Compress) -cnotmatch 'private|coordinates|payload')
}
foreach($code in @('Invalid','Limit','TargetChanged','Ungranted','Unavailable','Timeout','Cancelled','PartialInput','Io','RetirementPending')){
    $v=New-Receipt;$v.error=$code;Project $v;Assert ($diagnostic.native_error -ceq $code -and $diagnostic.native_capture_refusal -ceq 'color_depth')
}
foreach($code in @('invalid','consent','stale','own_window','unavailable','limit','color_depth','codec_missing','timeout','cancelled','busy','storage','platform','unsupported')){
    $v=New-Receipt;$v.capture_refusal=$code;Project $v;Assert ($diagnostic.native_capture_refusal -ceq $code)
}
$v=New-Receipt;$v.error=@{Platform=@{phase='private/path';code=4294967295}};Project $v;Assert ($diagnostic.native_error -ceq 'Platform')
$v=New-Receipt;$v.error=$null;$v.capture_refusal=$null;Project $v;Assert ($diagnostic.native_error -ceq 'none' -and $diagnostic.native_capture_refusal -ceq 'none')
foreach($bad in @('private/path','unavailable',@{Platform=@{};private='payload'},@{platform=@{}},42)){
    $v=New-Receipt;$v.error=$bad;Project $v;Assert ($diagnostic.native_error -ceq 'unknown')
}
foreach($bad in @('private/path','ColorDepth',@{private='payload'},42)){
    $v=New-Receipt;$v.capture_refusal=$bad;Project $v;Assert ($diagnostic.native_capture_refusal -ceq 'unknown')
}
foreach($key in @('schema','operation','sampling_scope','status','capture_session_id','sample_id','helper_sha256')){
    $v=New-Receipt;$v[$key]='private/path';Project $v;Assert ($diagnostic.native_error -ceq 'unknown' -and $diagnostic.native_capture_refusal -ceq 'unknown')
}
foreach($key in @('error','capture_refusal')){
    $v=New-Receipt;$v.Remove($key);Project $v;Assert ($diagnostic.native_error -ceq 'unknown' -and $diagnostic.native_capture_refusal -ceq 'unknown')
}
foreach($raw in @('invalid private json','{}',('x'*4194305))){
    $script:diagnostic=[ordered]@{native_error='unknown';native_capture_refusal='unknown'}
    Save-PaintFrameDiagnostic $raw $sample
    Assert ($diagnostic.native_error -ceq 'unknown' -and $diagnostic.native_capture_refusal -ceq 'unknown')
}
$expected=[ordered]@{
    'Actual mouse pointer must remain outside the owned Paint canvas'='cursor_in_canvas'
    'Native helper startup failed'='helper_startup'
    'Native helper did not complete bounded work'='helper_work'
    'Actual native WGC canvas sample refused'='frame_verdict'
    'Stable native blank baseline required'='baseline_stability'
    'Opaque sRGB baseline required'='baseline_color'
    'Native canvas is not the fresh white fixture'='baseline_white'
}
foreach($key in $expected.Keys){Assert ((Get-PaintRefusalCode $key) -ceq $expected[$key]);Assert ((Get-PaintRefusalCode ($key+' private/path')) -ceq 'unknown')}
Assert ((Get-PaintRefusalCode 'native helper startup failed') -ceq 'unknown')
Assert ((Get-PaintRefusalCode ('private/path '+('x'*10000))) -ceq 'unknown')
Write-Host ('Paint diagnostics: '+$cases+' pure fixture assertions passed; no UI/device/native helper execution')

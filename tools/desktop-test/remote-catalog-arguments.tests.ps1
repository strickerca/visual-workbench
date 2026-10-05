param([Parameter(Mandatory)][string]$BuildScript)
Set-StrictMode -Version Latest
$ErrorActionPreference='Stop'
$tokens=$null;$errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile($BuildScript,[ref]$tokens,[ref]$errors)
if($errors.Count -ne 0){throw 'Catalog build arguments require parsed source'}
$functions=@($ast.FindAll({param($n) $n -is [Management.Automation.Language.FunctionDefinitionAst] -and $n.Name -ceq 'Get-RemoteEditorCatalogArguments'},$false))
if($functions.Count -ne 1){throw 'Catalog build argument function census differs'}
. ([scriptblock]::Create($functions[0].Extent.Text))
$count=0
function Pass-Catalog([scriptblock]$body){& $body;$script:count++}
function Refuse-Catalog([string]$code,[scriptblock]$body){
    $caught=$null;try{& $body}catch{$caught=$_.Exception}
    if($null -eq $caught -or $caught.Message -cne $code){throw "Expected catalog argument refusal $code"}
    $script:count++
}
$root='C:\Repo';$digest='a'*64
Pass-Catalog {if(@(Get-RemoteEditorCatalogArguments '' '' $root).Count -ne 0){throw 'Absent catalog must add no arguments'}}
Pass-Catalog {$result=@(Get-RemoteEditorCatalogArguments '.local\actual.json' $digest $root);if($result.Count -ne 2 -or $result[0] -cne '-PvwRemoteEditorCatalog=C:\Repo\.local\actual.json' -or $result[1] -cne ('-PvwRemoteEditorCatalogSha256='+$digest)){throw 'Relative catalog must bind project root and exact hash'}}
Pass-Catalog {$result=@(Get-RemoteEditorCatalogArguments 'C:\Owned\actual.json' $digest $root);if($result.Count -ne 2 -or $result[0] -cne '-PvwRemoteEditorCatalog=C:\Owned\actual.json'){throw 'Exact admitted absolute catalog must remain unchanged'}}
Refuse-Catalog 'Root-admitted editor catalog path and SHA256 must be supplied together' {Get-RemoteEditorCatalogArguments 'file.json' '' $root}
Refuse-Catalog 'Root-admitted editor catalog path and SHA256 must be supplied together' {Get-RemoteEditorCatalogArguments '' $digest $root}
Refuse-Catalog 'Invalid root-admitted editor catalog SHA256' {Get-RemoteEditorCatalogArguments 'file.json' ('A'*64) $root}
Refuse-Catalog 'Invalid root-admitted editor catalog SHA256' {Get-RemoteEditorCatalogArguments 'file.json' ('a'*63) $root}
Refuse-Catalog 'Ambiguous editor catalog path' {Get-RemoteEditorCatalogArguments 'C:relative.json' $digest $root}
Refuse-Catalog 'Editor catalog requires an ordinary absolute local path' {Get-RemoteEditorCatalogArguments '\\server\share\actual.json' $digest $root}
Refuse-Catalog 'Editor catalog requires an ordinary absolute local path' {Get-RemoteEditorCatalogArguments 'C:\Owned\actual.json:stream' $digest $root}
[ordered]@{schema=1;operation='normal-catalog-pure-arguments';passed=$count;failed=0;build_native_device_executed=$false}|ConvertTo-Json -Compress

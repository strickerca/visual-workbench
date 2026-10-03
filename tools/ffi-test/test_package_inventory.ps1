#requires -Version 7.2
# Source-only fixture handoff. Executes the exact production function with a
# strict fake adb; it does not dot-source or run the instrumentation script.
Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
$source = Join-Path $PSScriptRoot 'run_shared_android.ps1'
$tokens = $null; $errors = $null
$ast = [Management.Automation.Language.Parser]::ParseFile($source, [ref]$tokens, [ref]$errors)
if ($errors.Count -ne 0) { throw 'Runner syntax failed fixture admission.' }
$functions = @($ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -ceq 'Get-TestPackagePath' }, $true))
if ($functions.Count -ne 1) { throw 'Expected one production inventory function.' }
. ([ScriptBlock]::Create($functions[0].Extent.Text))
$package = 'com.visualworkbench.shared.test'
$device = [pscustomobject]@{ Serial = 'fixture-only' }
$script:responses = [Collections.Generic.Queue[object]]::new()
$script:calls = [Collections.Generic.List[string]]::new()
function Invoke-VwAdb {
    param($Device, [string[]]$Arguments, [string]$Phase, [int]$TimeoutSeconds, [switch]$Capture)
    if ($Device.Serial -cne 'fixture-only' -or $TimeoutSeconds -ne 20 -or -not $Capture) { throw 'Unexpected mock invocation.' }
    $script:calls.Add(($Arguments -join ' '))
    if ($script:responses.Count -eq 0) { throw 'Unexpected additional adb operation.' }
    $reply = $script:responses.Dequeue()
    if ($reply.Fail) { throw 'Synthetic nonzero adb exit.' }
    return [pscustomobject]@{ Lines = $reply.Lines; ExitCode = 0 }
}
function Reply([string[]]$Lines, [bool]$Fail = $false) { return [pscustomobject]@{ Lines = $Lines; Fail = $Fail } }
function Check-Case([string]$Name, [object[]]$Replies, [bool]$Reject, [AllowNull()][object]$Expected, [int]$Calls) {
    $script:responses.Clear(); $script:calls.Clear()
    foreach ($reply in $Replies) { $script:responses.Enqueue($reply) }
    $failed = $false; $actual = $null
    try { $actual = Get-TestPackagePath } catch { $failed = $true }
    if ($failed -ne $Reject -or (-not $Reject -and $actual -cne $Expected) -or $script:calls.Count -ne $Calls -or $script:responses.Count -ne 0) { throw ('Inventory fixture failed: ' + $Name) }
    if ($script:calls[0] -cne ('shell pm list packages --user 0 ' + $package)) { throw 'Inventory was not explicitly owner-user scoped.' }
    if ($Calls -eq 2 -and $script:calls[1] -cne ('shell pm path --user 0 ' + $package)) { throw 'Path lookup was not explicitly owner-user scoped.' }
    Write-Host ('PASS package inventory fixture: ' + $Name)
}
$present = Reply @('package:com.visualworkbench.shared.test')
$path = Reply @('package:/data/app/fixture/base.apk')
Check-Case 'absent does not invoke failing pm path' @((Reply @())) $false $null 1
Check-Case 'exact present path' @($present, $path) $false '/data/app/fixture/base.apk' 2
Check-Case 'substring match is ambiguous' @((Reply @('package:com.visualworkbench.shared.test.other'))) $true $null 1
Check-Case 'duplicate inventory is ambiguous' @((Reply @('package:com.visualworkbench.shared.test', 'package:com.visualworkbench.shared.test'))) $true $null 1
Check-Case 'unexpected inventory output' @((Reply @('Error: synthetic'))) $true $null 1
Check-Case 'present then missing is ambiguous' @($present, (Reply @())) $true $null 2
Check-Case 'split package is refused' @($present, (Reply @('package:/data/app/fixture/base.apk', 'package:/data/app/fixture/split.apk'))) $true $null 2
Check-Case 'unexpected package location' @($present, (Reply @('package:/system/app/fixture/base.apk'))) $true $null 2
Check-Case 'inventory nonzero remains failure' @((Reply @() $true)) $true $null 1
Check-Case 'path nonzero remains failure' @($present, (Reply @() $true)) $true $null 2
Write-Host 'Package inventory fixtures: 10 passed; no adb or device calls performed.'

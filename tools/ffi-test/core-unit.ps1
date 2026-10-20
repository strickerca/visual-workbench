Set-StrictMode -Version Latest

# Cargo JSON is captured with the existing process runner's path redaction.
# Only a current exact lib-unit compiler artifact may name the executed file.
function Resolve-VwCoreUnitArtifact {
    param([string[]]$Lines,[string]$ProjectRoot)
    $root=[IO.Path]::GetFullPath($ProjectRoot).TrimEnd('\','/')
    $normalRoot=$root.Replace('\','/')
    $found=$null;$finished=0;$bytes=0L
    foreach($line in $Lines) {
        $bytes+=[Text.Encoding]::UTF8.GetByteCount($line)
        if($bytes -gt 67108864 -or $line.Length -gt 524288){throw 'Core unit compiler record exceeds bounds'}
        if(-not $line.StartsWith('{')){continue}
        try{$message=$line|ConvertFrom-Json -ErrorAction Stop}catch{throw 'Malformed core unit compiler JSON'}
        if($message.reason -eq 'build-finished') {
            if($message.success -isnot [bool] -or -not $message.success){throw 'Core unit build did not succeed'}
            $finished++;continue
        }
        if($message.reason -ne 'compiler-artifact' -or $message.target.name -ne 'vw_core'){continue}
        if($message.profile.test -isnot [bool] -or -not $message.profile.test){throw 'Core artifact is not the unit test'}
        if($null -ne $found){throw 'Duplicate core unit artifact'}
        if(@($message.target.kind).Count -ne 2 -or @($message.target.crate_types).Count -ne 2 -or
            @(Compare-Object @('cdylib','rlib') @($message.target.kind)).Count -ne 0 -or
            @(Compare-Object @('cdylib','rlib') @($message.target.crate_types)).Count -ne 0){throw 'Unexpected core library target kinds'}
        if($message.target.edition -ne '2024'){throw 'Unexpected core unit edition'}
        foreach($pair in @(@($message.manifest_path,'/core/crates/vw-ffi/Cargo.toml'),@($message.target.src_path,'/core/crates/vw-ffi/src/lib.rs'))) {
            $value=([string]$pair[0]).Replace('\','/')
            if($value -cne ('[redacted]'+$pair[1]) -and -not [string]::Equals($value,($normalRoot+$pair[1]),[StringComparison]::OrdinalIgnoreCase)){throw 'Core unit source binding differs'}
        }
        $exe=([string]$message.executable).Replace('\','/')
        $prefix=if($exe.StartsWith('[redacted]/',[StringComparison]::Ordinal)){'[redacted]'}else{$normalRoot}
        $tail='/target/debug/deps/'
        if(-not $exe.StartsWith($prefix+$tail,[StringComparison]::OrdinalIgnoreCase)){throw 'Core unit executable escapes the expected output'}
        $name=$exe.Substring(($prefix+$tail).Length)
        if($name -cnotmatch '^vw_core-[0-9a-f]{16}\.exe$'){throw 'Unexpected core unit executable name'}
        if(@($message.filenames|Where-Object{([string]$_).Replace('\','/') -ceq $exe}).Count -ne 1){throw 'Core unit executable is not in its artifact receipt'}
        $found=Join-Path $root ('target/debug/deps/'+$name)
    }
    if($finished -ne 1 -or $null -eq $found){throw 'Missing successful core unit artifact receipt'}
    return $found
}

function Assert-VwCoreUnitResult {
    param([string[]]$Lines,[string[]]$RequiredTests=@())
    $starts=@($Lines | Where-Object { $_ -cmatch '^running [1-9][0-9]* tests?$' })
    $ends=@($Lines | Where-Object { $_ -cmatch '^test result: ' })
    if($starts.Count -ne 1 -or $ends.Count -ne 1 -or
       $ends[0] -cnotmatch '^test result: ok\. ([1-9][0-9]*) passed; 0 failed; ([01]) ignored; 0 measured; [0-9]+ filtered out; finished in [0-9.]+s$'){
        throw 'Core unit test execution census missing, empty, failed, or unexpected ignored count'
    }
    $count=[int]$Matches[1];$ignored=[int]$Matches[2]
    if($starts[0] -cnotmatch '^running ([1-9][0-9]*) tests?$' -or [int]$Matches[1] -ne ($count+$ignored)){throw 'Core unit start/end census mismatch'}
    $names=@(foreach($line in $Lines){if($line -cmatch '^test ([A-Za-z0-9_:]+) \.\.\. ok$'){$Matches[1]}})
    if($names.Count -ne $count -or @($names|Sort-Object -Unique).Count -ne $count){throw 'Core unit passing-case census mismatch'}
    $caseLines=@($Lines | Where-Object {$_.StartsWith('test ',[StringComparison]::Ordinal) -and -not $_.StartsWith('test result: ',[StringComparison]::Ordinal)})
    $ignoredLines=@($caseLines | Where-Object {$_ -cmatch '^test [A-Za-z0-9_:]+ \.\.\. ignored(?:, .*)?$'})
    if($caseLines.Count -ne ($count+$ignored) -or $ignoredLines.Count -ne $ignored){throw 'Core unit case-status census mismatch'}
    if($ignored -eq 1){
        # This test deliberately retains a process-global permit in its child.
        # The ordinary wrapper must pass AND expose its post-child-success proof
        # in its own --show-output section; a bare ignored result is insufficient.
        $probe='giant_images::producer::picked::tests::quarantine_probe'
        $wrapper='giant_images::producer::picked::tests::cleanup_failure_retains_the_real_admission_in_an_isolated_process'
        $reason='Executed by the bounded quarantine wrapper; do not leak this process-global permit into unrelated tests'
        if($ignoredLines[0] -cne ('test '+$probe+' ... ignored, '+$reason) -or @($names | Where-Object {$_ -ceq $wrapper}).Count -ne 1){throw 'Core unit ignored probe lacks its passing isolation wrapper'}
        $header='---- '+$wrapper+' stdout ----'
        $marker='quarantine probe executed: 1 passed; same4MiB retained and third input refused'
        $headers=@(for($i=0;$i -lt $Lines.Count;$i++){if($Lines[$i] -ceq $header){$i}})
        $markers=@(for($i=0;$i -lt $Lines.Count;$i++){if($Lines[$i] -ceq $marker){$i}})
        if($headers.Count -ne 1 -or $markers.Count -ne 1 -or $markers[0] -le $headers[0]){throw 'Core unit quarantine child proof missing or duplicate'}
        # Libtest prints successful stdout after all case statuses and before
        # its final census. Refuse foreign sections or nonempty intervening text.
        $lastCase=-1;$end=-1
        for($i=0;$i -lt $Lines.Count;$i++){
            if($Lines[$i] -cin $caseLines){$lastCase=$i}
            if($Lines[$i] -ceq $ends[0]){$end=$i}
        }
        if($headers[0] -le $lastCase -or $markers[0] -ge $end){throw 'Core unit quarantine proof is outside successful stdout'}
        $successSections=0
        for($i=$lastCase+1;$i -lt $headers[0];$i++){
            if($Lines[$i] -ceq 'successes:'){$successSections++}
            if($Lines[$i] -ceq 'failures:'){throw 'Core unit quarantine proof is inside failure output'}
        }
        if($successSections -ne 1){throw 'Core unit quarantine successful stdout heading missing or duplicate'}
        for($i=$headers[0]+1;$i -lt $markers[0];$i++){if($Lines[$i] -cne ''){throw 'Core unit quarantine proof belongs to a foreign section'}}
    }
    foreach($name in $RequiredTests){if(@($names|Where-Object {$_ -ceq $name}).Count -ne 1){throw 'Required core regression did not execute'}}
    return $count
}

function Get-VwCoreUnitArguments {
    param([string]$TestFilter)
    # Process-global admission tests must be serialized. Successful stdout is
    # needed only as fixed proof for the exact wrapper-accounted ignored probe.
    $arguments=@('--test-threads=1','--show-output')
    if($TestFilter){$arguments=@($TestFilter)+$arguments}
    return $arguments
}

function Invoke-VwCoreUnitTest {
    param([string]$ProjectRoot,[int]$Limit,[string]$TestFilter,[string[]]$RequiredTests=@(),[switch]$IntegrationCarrierFault)
    # This path is the native Windows test lane. Cross-target overrides refuse;
    # Android's separate build command is unchanged.
    if($env:CARGO_BUILD_TARGET){throw 'Core unit test requires the selected Windows MSVC host'}
    $tool=Invoke-VwProcess -FilePath 'rustc.exe' -ArgumentList @('+1.99.0','--version','--verbose') -WorkingDirectory $ProjectRoot -Phase 'test-ffi-core-unit-toolchain' -TimeoutSeconds 30 -Capture
    if($tool.ExitCode -ne 0 -or @($tool.Lines|Where-Object{$_ -ceq 'host: x86_64-pc-windows-msvc'}).Count -ne 1){throw 'Core unit test requires the pinned Windows MSVC host'}
    $buildArgs=@('+1.99.0','rustc','--locked','-p','vw-ffi','--lib','--profile','test','--message-format=json')
    if($IntegrationCarrierFault){$buildArgs+=@('--features','integration-carrier-fault')}
    $buildArgs+=@('--','-C','link-arg=/DEBUG:NONE')
    $build=Invoke-VwProcess -FilePath 'cargo.exe' -ArgumentList $buildArgs -WorkingDirectory $ProjectRoot -Phase 'build-ffi-core-unit' -TimeoutSeconds $Limit -Capture -CleanCompilerTelemetry
    if($build.ExitCode -ne 0){throw "Core unit build failed (exit $($build.ExitCode)); see its retained external text log"}
    $exe=Resolve-VwCoreUnitArtifact -Lines $build.Lines -ProjectRoot $ProjectRoot
    $cursor=[IO.Path]::GetFullPath($exe)
    while($cursor) {
        if(-not(Test-Path -LiteralPath $cursor)){throw 'Core unit output path is absent'}
        if((Get-Item -LiteralPath $cursor -Force).Attributes -band [IO.FileAttributes]::ReparsePoint){throw 'Core unit output path is redirected'}
        $parent=[IO.Path]::GetDirectoryName($cursor)
        if($parent -eq $cursor){break};$cursor=$parent
    }
    $lease=[IO.File]::Open($exe,[IO.FileMode]::Open,[IO.FileAccess]::Read,[IO.FileShare]::Read)
    try {
        $sha=[Security.Cryptography.SHA256]::Create()
        try{$before=[BitConverter]::ToString($sha.ComputeHash($lease)).Replace('-','').ToLowerInvariant();$lease.Position=0
            Write-Host "[test-ffi-core-unit] exact executable SHA256 $before"
            $testArgs=@(Get-VwCoreUnitArguments -TestFilter $TestFilter)
            $run=Invoke-VwProcess -FilePath $exe -ArgumentList $testArgs -WorkingDirectory $ProjectRoot -Phase 'test-ffi-core-unit' -TimeoutSeconds $Limit -Capture
            $run.Lines|Write-Host
            $lease.Position=0;$after=[BitConverter]::ToString($sha.ComputeHash($lease)).Replace('-','').ToLowerInvariant()
            if($after -cne $before){throw 'Core unit executable changed during execution'}
            if($run.ExitCode -ne 0){throw "Core unit tests failed (exit $($run.ExitCode)); see their retained external text log"}
            $count=Assert-VwCoreUnitResult -Lines $run.Lines -RequiredTests $RequiredTests
            Write-Host "[test-ffi-core-unit] validated executed cases=$count"
        }finally{$sha.Dispose()}
    }finally{$lease.Dispose()}
}

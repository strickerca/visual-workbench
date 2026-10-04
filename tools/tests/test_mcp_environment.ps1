$ErrorActionPreference='Stop'
$repository=Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
. (Join-Path $repository 'tools/mcp-build.ps1')
$owned=Join-Path ([IO.Path]::GetTempPath()) ('vw-mcp-env-fixture-'+[Guid]::NewGuid().ToString('N'))
$projectRoot=$owned
$saved=@{}
foreach($item in @(Get-ChildItem Env: | Where-Object {$_.Name -imatch '^npm_config_' -or $_.Name -in @('NODE_OPTIONS','NODE_PATH')})) {
    $saved[$item.Name]=$item.Value;Remove-Item -LiteralPath ('Env:'+$item.Name)
}
function Must-Refuse([scriptblock]$Body) {
    $refused=$false
    try { & $Body } catch { if($_.Exception.Message -notlike 'MCP build refuses*'){throw};$refused=$true }
    if(-not $refused){throw 'An inherited tool override was admitted'}
}
try {
    [IO.Directory]::CreateDirectory((Join-Path $owned 'mcp')) | Out-Null
    Assert-McpBuildEnvironment (Join-Path $owned 'mcp')
    foreach($name in @('npm_config_dry_run','NPM_CONFIG_IGNORE_SCRIPTS','NODE_OPTIONS','NODE_PATH')) {
        [Environment]::SetEnvironmentVariable($name,'true','Process')
        try { Must-Refuse { Assert-McpBuildEnvironment (Join-Path $owned 'mcp') } }
        finally { Remove-Item -LiteralPath ('Env:'+$name) -ErrorAction SilentlyContinue }
    }
    foreach($file in @((Join-Path $owned '.npmrc'),(Join-Path $owned 'mcp/.npmrc'))) {
        [IO.File]::WriteAllText($file,'dry-run=true')
        try { Must-Refuse { Assert-McpBuildEnvironment (Join-Path $owned 'mcp') } }
        finally { Remove-Item -LiteralPath $file -Force }
    }
    $hadManifest=Test-Path -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST'
    $oldManifest=[Environment]::GetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST','Process')
    try {
        Remove-Item -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST' -ErrorAction SilentlyContinue
        Invoke-McpManifestScope -Manifest $null -Body {
            if(Test-Path -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST'){throw 'Absent manifest became an empty override'}
        }
        if(Test-Path -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST'){throw 'Absent manifest was not restored'}
        [Environment]::SetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST','retained-fixture','Process')
        Invoke-McpManifestScope -Manifest $null -Body {
            if(Test-Path -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST'){throw 'Server build retained a manifest override'}
        }
        if($env:VW_MCP_APP_IMAGE_MANIFEST -cne 'retained-fixture'){throw 'Existing manifest was not restored'}
        try {
            Invoke-McpManifestScope -Manifest 'selected-fixture' -Body {
                if($env:VW_MCP_APP_IMAGE_MANIFEST -cne 'selected-fixture'){throw 'Selected manifest was not installed'}
                throw 'expected-fixture-failure'
            }
        } catch { if($_.Exception.Message -cne 'expected-fixture-failure'){throw} }
        if($env:VW_MCP_APP_IMAGE_MANIFEST -cne 'retained-fixture'){throw 'Failed build did not restore the manifest'}
    } finally {
        if($hadManifest){[Environment]::SetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST',$oldManifest,'Process')}
        else{Remove-Item -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST' -ErrorAction SilentlyContinue}
    }
    Write-Host 'MCP environment fixtures: PASS (six refusal cases and three manifest lifetime cases; no npm or child process executed)'
} finally {
    foreach($name in $saved.Keys){[Environment]::SetEnvironmentVariable($name,$saved[$name],'Process')}
    if(Test-Path -LiteralPath $owned){
        $resolved=(Resolve-Path -LiteralPath $owned).Path
        if($resolved -cne [IO.Path]::GetFullPath($owned) -or -not $resolved.StartsWith([IO.Path]::GetTempPath(),[StringComparison]::OrdinalIgnoreCase)){throw 'Fixture cleanup containment refused'}
        Remove-Item -LiteralPath $resolved -Recurse -Force
    }
}

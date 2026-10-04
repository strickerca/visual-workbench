# Loaded by build.ps1; all child execution uses its bounded process-tree runner.
function Invoke-McpManifestScope {
    param([AllowNull()][string]$Manifest, [scriptblock]$Body)
    $wasPresent = Test-Path -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST'
    $previous = [Environment]::GetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST','Process')
    try {
        # PowerShell/.NET versions differ on whether a null setter removes an
        # environment entry or retains an empty value. Cargo distinguishes them.
        if ([string]::IsNullOrEmpty($Manifest)) {
            Remove-Item -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST' -ErrorAction SilentlyContinue
        } else {
            [Environment]::SetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST',$Manifest,'Process')
        }
        & $Body
    } finally {
        if ($wasPresent) {
            [Environment]::SetEnvironmentVariable('VW_MCP_APP_IMAGE_MANIFEST',$previous,'Process')
        } else {
            Remove-Item -LiteralPath 'Env:VW_MCP_APP_IMAGE_MANIFEST' -ErrorAction SilentlyContinue
        }
    }
}

function Assert-McpBuildEnvironment {
    param([string]$Directory)
    foreach ($name in @('NODE_OPTIONS','NODE_PATH')) {
        if ([Environment]::GetEnvironmentVariable($name)) { throw "MCP build refuses $name overrides" }
    }
    if (@(Get-ChildItem Env: | Where-Object { $_.Name -imatch '^npm_config_' }).Count -ne 0) {
        throw 'MCP build refuses inherited npm configuration overrides'
    }
    if ((Test-Path -LiteralPath (Join-Path $Directory '.npmrc')) -or
        (Test-Path -LiteralPath (Join-Path $projectRoot '.npmrc'))) {
        throw 'MCP build refuses project npm configuration overrides'
    }
}

function Get-McpNodeDirectory {
    Assert-McpBuildEnvironment (Join-Path $projectRoot 'mcp')
    $directory = Join-Path $projectRoot '.local/toolchains/node-v24.21.0-win-x64'
    Run-Step 'license-mcp-node' 'python.exe' @('tools/check_npm_licenses.py','--node',$directory)
    return $directory
}

function Install-McpDependencies {
    param([string]$Directory, [string]$NodeDirectory, [switch]$Production)
    Assert-McpBuildEnvironment $Directory
    $configDirectory = Join-Path $projectRoot '.local/mcp-npm-config'
    [IO.Directory]::CreateDirectory($configDirectory) | Out-Null
    $configs = @('user.npmrc','global.npmrc') | ForEach-Object {
        $path = Join-Path $configDirectory $_
        if (Test-Path -LiteralPath $path) {
            if ((Get-Item -LiteralPath $path).Length -ne 0) { throw 'MCP npm configuration is not empty' }
        } else { [IO.File]::WriteAllBytes($path,[byte[]]@()) }
        $path
    }
    $arguments = @((Join-Path $NodeDirectory 'node_modules/npm/bin/npm-cli.js'),'ci','--ignore-scripts',
        '--no-audit','--no-fund','--offline','--cache',(Join-Path $projectRoot '.local/npm-cache'),
        '--userconfig',$configs[0],'--globalconfig',$configs[1])
    if ($Production) { $arguments += '--omit=dev' }
    Run-Step 'mcp-npm-ci' (Join-Path $NodeDirectory 'node.exe') $arguments $Directory
    $checks = @('tools/check_npm_licenses.py','--installed',$Directory,'--node',$NodeDirectory)
    if ($Production) { $checks += '--production' }
    Run-Step 'license-mcp-installed' 'python.exe' $checks
}

function Test-McpRuntime {
    Run-Step 'test-mcp-environment' 'pwsh.exe' @('-NoProfile','-File','tools/tests/test_mcp_environment.ps1')
    $nodeDirectory = Get-McpNodeDirectory
    Install-McpDependencies (Join-Path $projectRoot 'mcp') $nodeDirectory
    $tests = @(Get-ChildItem -LiteralPath (Join-Path $projectRoot 'mcp/test') -File -Filter '*.test.mjs' |
        Sort-Object Name | ForEach-Object { $_.FullName })
    $tests += Join-Path $projectRoot 'mcp/codex/test_contract.mjs'
    Run-Step 'test-mcp-node' (Join-Path $nodeDirectory 'node.exe') (@('--test','--test-concurrency=2') + $tests)
    Run-Step 'test-mcp-package-runtime' 'python.exe' @('-m','unittest','discover','-s','mcp','-p','test_*.py','-v')
}

function Build-McpServerResources {
    param([string]$Nonce)
    if ($Nonce -cnotmatch '^[0-9a-f]{32}$') { throw 'MCP build nonce refused' }
    $nodeDirectory = Get-McpNodeDirectory
    $work = Join-Path $projectRoot ('.local/mcp-build/' + $Nonce)
    if (Test-Path -LiteralPath $work) { throw 'MCP build requires a fresh staging directory' }
    [IO.Directory]::CreateDirectory((Join-Path $work 'stage/mcp')) | Out-Null
    foreach ($name in @('package.json','package-lock.json')) {
        Copy-Item -LiteralPath (Join-Path $projectRoot ('mcp/' + $name)) -Destination (Join-Path $work ('stage/mcp/' + $name)) -ErrorAction Stop
    }
    Install-McpDependencies (Join-Path $work 'stage/mcp') $nodeDirectory -Production
    Invoke-McpManifestScope -Manifest $null -Body {
        Run-Cargo 'build-mcp-server-native' @('build','--locked','-p','vw-mcp-native','--bin','vw-mcp-host','--bin','vw-mcp-package','--bin','vw-codex-host')
    }
    Run-Step 'stage-mcp-runtime' 'python.exe' @('tools/stage_mcp_runtime.py',$work,$nodeDirectory)
    Run-Step 'inventory-mcp-runtime' 'python.exe' @('mcp/package_runtime.py','server',(Join-Path $work 'stage'),(Join-Path $work 'resources'))
    return (Join-Path $work 'resources')
}

function Complete-McpApplicationImage {
    param([string]$Nonce)
    $image = Join-Path $projectRoot 'apps/desktop/build/compose/binaries/main/app/VisualWorkbenchDev'
    $manifest = Join-Path $projectRoot ('.local/mcp-build/' + $Nonce + '/app-image.sha256')
    Run-Step 'inventory-mcp-application' 'python.exe' @('mcp/package_runtime.py','app-image',$image,$manifest,'--launcher','VisualWorkbenchDev.exe')
    Invoke-McpManifestScope -Manifest $manifest -Body {
        Run-Cargo 'build-mcp-final-bridge' @('build','--locked','-p','vw-mcp-native','--bin','vw-mcp')
        [IO.File]::Copy((Join-Path $projectRoot 'target/debug/vw-mcp.exe'),(Join-Path $image 'vw-mcp.exe'),$false)
    }
}

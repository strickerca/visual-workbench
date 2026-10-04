# Project-local MCP runtime setup

The current checkout has a verified Node.js runtime and populated npm cache.
Desktop builds use those project-local inputs; a machine-wide Node installation
is not required. The runtime shipped in the desktop app image is independently
inventoried and verified before execution.

For a fresh checkout, acquire the official
[Node.js 24.21.0 Windows x64 archive](https://nodejs.org/dist/v24.21.0/node-v24.21.0-win-x64.zip).
Its reviewed SHA-256 is
`158f7685b44de51f6c0df1d153526cbcd3e1bc739a8dfc607721cef75de9e541`.
The initial acquisition verified the official detached SHASUMS signature against
the pinned Node release-key repository; the key, checksum and signature input
hashes are recorded in `tools/licenses/node-runtime.json`. Do not accept a
different archive by updating this hash automatically.

Extract the verified archive to `.local/toolchains/node-v24.21.0-win-x64` under
the checkout. Preserve its complete `LICENSE`, `node.exe` and `node_modules/npm`
tree. Run the offline admission check before executing that Node or npm:

```powershell
python tools/check_npm_licenses.py --node .local/toolchains/node-v24.21.0-win-x64
```

Populate `.local/npm-cache` through the pinned npm CLI using `npm ci` in `mcp/`
with the committed `package-lock.json`, `--ignore-scripts --no-audit --no-fund`,
and that exact project-local cache. Use empty project-owned user and global
configuration files and the same clean environment enforced by
`tools/mcp-build.ps1`: no `NODE_OPTIONS`, `NODE_PATH`, inherited `npm_config_*`,
or project `.npmrc`. The initial cache acquisition requires network access to
the lockfile's exact `https://registry.npmjs.org/` artifact URLs. npm verifies
each lockfile SHA-512 integrity value. Do not run install hooks or regenerate the
lock as a setup shortcut.

After acquisition, normal build/test entry points perform scripts-disabled
**offline** `npm ci` and validate the installed graph, metadata and full notices
against `tools/licenses/reviewed-npm.json`. The source tests use 20 packages;
desktop packaging installs only the nine production packages. A missing cache
entry or changed tool, package, graph or notice causes refusal.

```powershell
pwsh -NoProfile -File build.ps1 test-mcp -TimeoutSeconds 1200
pwsh -NoProfile -File build.ps1 build-desktop-distribution -TimeoutSeconds 1800
pwsh -NoProfile -File tools/desktop-test/run.ps1 -Execute -Mode startup
pwsh -NoProfile -File tools/desktop-test/run.ps1 -Execute -Mode mcp -TimeoutSeconds 180
```

The last command checks local packaged service start/stop/restart and ownership.
It does not send a message, grant capture, contact an external client or pass a
product gate. Actual Claude/Code and Codex workflows require their explicit
owner actions and separately recorded evidence. The app stores its loopback
bearer in Windows Credential Manager; no token belongs in project files or
client configuration.

# Dependency evidence and artifact gate

Reviewed via native web and centrally resolved on 2026-10-03. The table records
upstream provenance; exact artifact inventories and integrity receipts are
recorded in `tools/licenses/reviewed-npm.json` and `node-runtime.json`.

| Component | Exact selection | Primary evidence | License evidence |
|---|---|---|---|
| `@modelcontextprotocol/server`, `core`, test `client` | 2.3.0 | [Official release](https://github.com/modelcontextprotocol/typescript-sdk/releases/tag/v2.3.0), signed release commit short ID `a202a36` | Release manifests declare Apache-2.0; [upstream license](https://raw.githubusercontent.com/modelcontextprotocol/typescript-sdk/main/LICENSE) retains MIT contributions during the transition. Ship the actual artifact's full notices. |
| `ajv` | 8.17.1 | SDK's [workspace catalog](https://raw.githubusercontent.com/modelcontextprotocol/typescript-sdk/main/pnpm-workspace.yaml) | [MIT](https://raw.githubusercontent.com/ajv-validator/ajv/v8.17.1/LICENSE) |
| `ajv-formats` | 3.0.1 | Same SDK catalog | [MIT](https://raw.githubusercontent.com/ajv-validator/ajv-formats/v3.0.1/LICENSE) |
| Packaged Node x64 | 24.21.0 | [Official current LTS download](https://nodejs.org/en/download) | [Release license and bundled notices](https://raw.githubusercontent.com/nodejs/node/v24.21.0/LICENSE) |

The [tagged SDK protocol guide](https://raw.githubusercontent.com/modelcontextprotocol/typescript-sdk/v2.3.0/docs/protocol-versions.md)
expressly supports both required versions from the same factory: stateless
`createMcpHandler` for HTTP and `serveStdio` for both connection eras. The
2026-07-28 client tests pin that exact revision, so a fallback cannot count as a
pass. The legacy tests restrict supported versions to 2025-11-25.
[Official handler source](https://raw.githubusercontent.com/modelcontextprotocol/typescript-sdk/main/packages/core-internal/src/shared/protocol.ts)
supplies `context.mcpReq.signal`; SDK errors are not forwarded to logs.

[Claude's official channel reference](https://code.claude.com/docs/en/channels-reference)
documents `claude/channel`, stdio notifications, and the owner development
allowlist flag. A completed notification write does not establish agent receipt.

Native dependencies reuse existing repository pins: serde 1.0.229, serde_json
1.0.151, getrandom 0.4.3, zeroize 1.9.0, fs2 0.4.3, windows-sys 0.61.2 and test
tempfile 3.27.0. The only local path dependency is exact vw-package 0.1.0.
Kotlin uses the existing coroutines/JDK/JUnit runtime, with no new Gradle library.

The central lane resolved the npm lock with tarball SHA-512 integrity and full
transitive notices, then ran scripts-disabled offline `npm ci`. Node's signed
[SHASUMS256](https://nodejs.org/dist/v24.21.0/SHASUMS256.txt), x64 archive,
executable and notices were verified before use. Packaging generates the full
server file-hash map as an application resource. Development packages are not
release-signed artifacts; release signing remains a separate gate.

Central integration subsequently resolved and verified these inputs. The exact
twenty-package lock and notices are now bound by
`tools/licenses/reviewed-npm.json`; nine packages belong to the production graph.
`tools/licenses/node-runtime.json` records the verified official detached
signature, immutable release-key commit, archive/executable/notice hashes and
build-tool inventory. The offline gate validates extracted tool bytes before
execution. `build.ps1 test-mcp` uses the pinned runtime, scripts-disabled offline
installation and both official protocol-version fixtures. These checks do not
establish delivery to an external Claude or Codex session or an image profile.

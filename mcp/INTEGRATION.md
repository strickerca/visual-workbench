# Parent-only integration handoff

No existing Main, Gradle, Cargo workspace, lockfile, build entry point, ledger or
manifest was modified. The following edits are still necessary in the canonical
checkout after source review; they are not implied by copying this directory.

1. Import reviewed vw-package R2 and its instructions/semantics dependencies.
   Add `mcp/native` to the root Cargo workspace. It intentionally references
   `../../core/crates/vw-package` relative to its crate; no placeholder raster or
   package implementation is supplied. Build the three Windows binaries in the
   central lane after license/lock gates. Include the source tests in bounded
   root validation and strict lint/format checks.
2. Resolve/review npm artifacts as described in `DEPENDENCIES.md`, then use the
   exact Node 24.21.0 runtime to run `mcp/test/*.test.mjs`. Keep test-only client
   dependencies out of the production bundle. Test SDK routes before relying on
   any current API assumption; no installed SDK was available to this author.
3. Package `VisualWorkbench.exe`, `vw-mcp-host.exe`, `vw-mcp.exe`,
   `vw-mcp-package.exe`, `node.exe`, `mcp/src/**`, production `mcp/node_modules/**`
   and notices in a dedicated immutable runtime directory. Produce a trusted
   exact path→SHA-256 inventory as an app resource. `McpRuntime.verify` refuses
   unlisted files, redirects, mismatches and missing required binaries. Perform
   this I/O off Swing. Preserve the original signed notices separately if the
   packaging system requires them. Do not use a mutable owner-supplied manifest
   as the trusted hash source.
4. In existing desktop Main startup, recognize **only** `--mcp-minimized` as the
   bridge mode and use the app's single-instance handoff. It must create the
   same lifetime-owned MCP service and remain minimized without blocking owner
   capture/grant indicators. This foundation never opens SQLite independently.
   On quit, await `DesktopMcpHost.shutdown()` before releasing project/capture
   callbacks. On owner disconnect/revoke, cancel owned capture producers and
   settle their native cleanup. No global settings change is necessary.
5. Create one user-private, non-synchronized MCP state directory with the app's
   existing private storage policy. Call `DesktopMcpHost.open(verifiedRuntime,
   stateDirectory, ownerActions)`. Its cancellation handoff retains ownership of
   a late-created native host. The service owns bounded callbacks and a private
   writer/reader pool. Shutdown kills/reaps the supervisor before closing an
   inherited pipe that a blocked writer could hold; unconfirmed process death
   retains the single-service admission permit. Do not wrap this returned-handle call in a cancellable
   `withContext` boundary that can discard the acquired service.
6. Implement `McpOwnerActions.capture`: validate owner selector→native target,
   show/acknowledge the active indicator, perform grant-bound lossless capture,
   compile an exact package, atomically publish its private directory, then
   await `host.publish` before returning `McpCaptureReceipt`. Check cancellation
   before each publication boundary and retain/settle actual native ownership.
   An absent capture subsystem must return refusal, never `lossless=true`.
   Implement `submitResult` with exact package manifest binding, bounded PNG
   decode/admission and atomic untrusted Compare-inbox persistence. No result
   callback changes the canonical project or runs the returned text.
7. Existing owner UI must display each authenticated bridge PID/connection,
   exact grant selector list/expiry, revoke controls, active capture indicator,
   and a Send preview. Call `publish` for staging, and `pushClaude` only on the
   distinct explicit Send action. Its result means written, not delivered.
   Do not add a Codex send button until installed schema/runtime checks and the
   selected session's explicit owner action are wired.
8. Client configuration contains only the absolute `vw-mcp.exe` command and an
   empty argument list. It contains no HTTP credential, project path or token.
   Optional HTTP clients must load the fixed Credential Manager credential in
   their own protected native adapter; never paste it into a config. This
   candidate provides the protected HTTP server, not that optional consumer.

Central test order: pure Node admission/fixtures → both official SDK protocol
eras → Rust helper and Kotlin codec suites → real compiled package fixture →
Windows named-pipe ACL/client PID/reconnect/5th-client refusal/port persistence →
cancelled open/slow writes/app crash with child census → actual client pull and
owner grant/capture/Compare/push UI. Preserve text-only receipts and source/binary
hash bindings; any verification screenshots follow the owner's disposal rule.

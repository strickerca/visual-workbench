# MCP runtime (T2.09–T2.10)

The runtime is integrated into the desktop build. Native compilation, both
official SDK protocol-version fixtures, Codex schema/admission tests and the
dependency gates have run centrally. Packaged service startup and external
client acceptance are tracked separately in the phase evidence. These checks
do not establish a live Claude or Codex delivery. See `docs/MCP_SETUP.md` for
the project-local build inputs and commands.

The desktop owns `vw-mcp-host.exe`, which creates an SID-only, local-only Windows
named pipe. `vw-mcp.exe` forwards bounded stdio frames through that pipe, verifies
the server process has the same SID, and starts only the sibling packaged
`VisualWorkbench.exe --mcp-minimized` if the app is absent. Readiness expires
after 15 seconds. The bridge never opens project SQLite files or reads a token.
The app implements single-instance ownership and the explicit minimized mode.

The native supervisor creates/reads the fixed current-user Credential Manager
target `VisualWorkbench/MCP/LoopbackBearer/v1`. The random 256-bit bearer travels
only over the SDK child's inherited stdin. No bearer appears in arguments,
environment, project files, client configuration or logs. The native credential
wrapper and dedicated init serialization buffer are wiped, including partial
serialization/write failures; init borrows the credential without a JSON Value
clone. JavaScript strings, allocator/serializer internals and OS-internal copies
cannot promise zeroization.
The first successful bind selects a loopback port and fsyncs its decimal value
in `mcp-port-v1`; subsequent starts require that exact port. A busy, corrupt or
unreadable state fails without selecting another port. A held owner-file lock
serializes starts and credential initialization.

The Node child binds only `127.0.0.1`, requires an exact Host and constant-time
bearer comparison, and accepts either no Origin (native clients) or its own
exact Origin. `null`, aliases, foreign origins, duplicate security headers,
compressed bodies, upgrade requests and non-POST operations are refused.
HTTP is stateless request/response; parked subscriptions are refused. HTTP
capture is also refused: an arbitrary per-request client name cannot inherit
another agent's capture grant. The recommended token-free transport is stdio.

The official SDK factory exposes six tools and `vw://package/{id}`. It serves
both 2025-11-25 legacy initialization and 2026-07-28 per-request metadata through
the official era routers. We did not implement a homemade version handshake.
`DEPENDENCIES.md` distinguishes verified upstream support from unrun local tests.

Only the inherited desktop channel can publish an already compiled package,
grant/revoke capture, or push a Claude notification. Pipe identities are random,
listener-assigned and accompanied by an OS-checked bridge PID. Grants bind exact
selector IDs, one connection and a maximum ten-minute lifetime. Revocation,
replacement, expiry and disconnect cancel in-flight capture authority. The
capture callback must activate the visible capture indicator, obtain a lossless
frame, compile and publish its immutable package, and settle cancellation before
returning. There is no callback that silently reports an unimplemented capture
as successful. `submit_result` requires an existing published package and asks
the app to validate/persist an untrusted Compare inbox item; it never edits the
project, runs text, or grants capture.

Every read invokes the actual `vw_package::Package::from_files` verifier in an
owned bounded subprocess. That verifier checks exact inventory, schema,
revision/source bindings, hashes, PNG receipts and profiles. No arbitrary
agent-provided filesystem path is exposed. Immediately before delivery, files
are re-read within bounds and compared with their verified SHA-256. Packages are
immutable by ID/target. Ambiguous ID-only reads across multiple target variants
are refused; the UI should publish one target variant per package ID. JSON
integers beyond JavaScript's exact range are refused, never rounded silently.
Untrusted text remains literal package data.

Admission limits: four pipe agents, four tool operations, eight pending requests
per pipe, 6 MiB requests, 16 MiB results, 17 MiB internal frames, 4 MiB per image,
9 MiB aggregate encoded image bytes, 64 published packages and two verifier
children. A package verifier admits at most 32 MiB of files, 64 markers (69 files,
including clean/overview/semantic/manifest/prompt) and a
256 MiB raster working budget. Oversize responses require a file export fallback;
they are never silently downscaled. IPC queues are bounded, timed writes close
the owned endpoint, and native shutdown owns the SDK and verifier process tree
through a kill-on-close Job. A bridge blocked writing inherited stdout has one
15-second watchdog that exits only that bridge, closing its pipe and grants.
Desktop shutdown terminates/reaps its supervisor before closing pipe streams,
so a blocked writer cannot prevent termination. Native package verification is synchronous within
that child; cancellation kills/reaps the exact child, rather than claiming a
mid-codec cooperative cancellation boundary.

T2.10 has a narrow Claude channel implementation: an explicit desktop action
re-verifies a Claude package and emits `notifications/claude/channel` with a
resource ID/hash. Publication alone does not send. The receipt says only
`written_to_transport`, with `agent_delivery_confirmed=false`. Claude's research
preview owner flag and any organization policy still apply. No permission-relay
capability is declared. Codex handoff validates the installed app-server schema,
requires a selected idle existing thread, and shows a separate Send preview.
Image sending additionally requires a reviewed image profile; the shipped
profile inventory is empty, so unsupported image handoffs remain disabled.

The central `test-mcp` run passed 46 Node cases across the MCP server and Codex
adapter, including official SDK clients in both eras over HTTP and stdio.
Native and Kotlin coverage also exercise framing, package readers, process
ownership and shutdown. Lifecycle fixtures exercise oversized
input before spawn, post-spawn failure retaining child slots, a blocked inherited
writer, and a watchdog independent of blocked output. The native boundary test
compiles a genuine 64-marker package and verifies all 69 files. Other package test
doubles cover the adapter's immutable read boundary only; canonical package
correctness must also be centrally executed.

Dependency integrity and runtime file inventories are enforced before execution;
the packaged bundle includes every listed MCP file. Remaining acceptance includes
the packaged Windows service lifecycle, actual Claude and Codex pull/push,
real capture grants and indicators, optional native HTTP consumer integration,
and installed-client image profiles. External sends and paid provider actions
have not been performed by the automated validation. Passing adapter fixtures
does not establish those external or owner-operated scenarios.

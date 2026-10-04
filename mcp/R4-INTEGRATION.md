# MCP R4 narrow source repairs

R3's nine manifest files and original manifest are preserved exactly at the
workspace-owned `.handoffs/t2-mcp-r3-preserved-20261003`; R2 is also unchanged in
its prior handoff. R4 has no dependency, token, grant, schema or runtime-pin change.

Claude preview equality now validates the exact top-level and nested metadata
field sets, string types and values independent of JSON object insertion order.
The bounded shown record is copied through the production JSON admission before
any verifier suspension. This admits the native supervisor's recursively sorted
serde maps while refusing extra fields, changed nested metadata or new content.
It preserves explicit owner Send and the transport-only delivery receipt.

`McpOwnerWork` now owns the four callback slots/map until coroutine completion,
with its handler registered after map insertion and before dispatch. Cancellation
before the body starts sends one negative settlement and releases that exact
entry. Cancellation during a native action retains ownership until the action's
required settlement finishes. Failed reply delivery closes the host instead of
claiming the peer drained; shutdown atomically seals callback admission before
cancelling/joining its snapshot. Every native owner callback must still join its
native work when cancelled. No timeout or cancellation authorizes file removal.

Import the new production `McpOwnerWork.kt` and test `McpOwnerWorkTest.kt` along
with the existing R3 nine-file increment. The separate exact-target capture
manifest is unchanged. One new JavaScript test (25 total) exercises the actual
production Send path with supervisor-shaped JSON and extra/changed fields.
Three new Kotlin source tests exercise undispatched cancellation at full four-slot
capacity, retained cancellation while native work is blocked, and an already
cancelled parent plus reply failure. Existing Owner drain tests bind negative
settlement to actual release of the JS pending slots. All new tests remain unrun;
this is source repair, not runtime/platform/client acceptance.

# MCP R3: removal drain and exact owner Send (source candidate)

R2's exact 31 files are preserved at the workspace-owned immutable handoff
`.handoffs/t2-mcp-r2-preserved-20261003`. Its original 17-case central SDK receipt
remains unchanged; it does not validate this increment. No runtime/dependency
pin, token/configuration behavior, HTTP Origin gate, client, or capture grant was
changed. No external client was configured or messaged.

This delta adds private owner commands unpublish and preview/claude to both the
native supervisor whitelist and actual JS/desktop facade. All package consumers
hold bounded reader leases through verifier/file work (including resources and
submit_result); removal hides the exact ID/target/hash immediately, then waits
for readers and actual native owner callbacks to settle. Late cancelled owner
calls retain their admission until an owner reply, rather than freeing capacity
and allowing deletion during a still-running native callback. Unknown settlement
or timeout is refusal, never permission to remove files. Exact recent removal
retries are bounded to 64 receipts; an older unknown receipt refuses. A mutation
fence rejects publications started before removal so delayed verification cannot
resurrect a retired entry. The parent catalog controller must serialize same-ID
publish/remove, release UI/drag leases and invoke native retire only after the
full successful unpublish/readers_drained/owner_callbacks_drained receipt.

previewClaude returns an immutable bounded content/meta/folder disclosure.
pushClaude now takes that exact displayed record plus manifest hash and compares
it to a freshly verified current package before writing the actual Claude channel
notification. The payload includes package/revision/marker summary, the folder,
resource URI and untrusted-text notice; no automatic Send is added. Its outcome
continues to mean written_to_transport=true, agent_delivery_confirmed=false.
Claude channel flags/admin policy and actual client acceptance remain untested.
Unknown Codex schema support stays refused; no speculative turn/start was added.

Seven new JS cases are written (24 total including the unchanged 17): held-reader
removal; timeout/retry; delayed publication fence; native-owner drain refusal;
exact displayed Claude payload; abandoned native-call capacity; owner-close
uncertainty. They are source-only and have not been executed by this author.
The existing process teardown, secret-handling, native package helper and both
protocol eras require central reruns on these exact bytes. No API is a claim of
complete desktop service/settings/capture/inbox/Send UI wiring; those are the
next separate integration files and literal root patches.

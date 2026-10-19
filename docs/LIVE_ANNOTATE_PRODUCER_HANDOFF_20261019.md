# T3.02 live-annotate capture producer handoff

Base: `6ae93d495fdfffd511f1e882facb80aaa0ab2dd4` on
`cloud/roadmap-source-integration-20261018`. This candidate changes only the
G-granted producer leaves. It does not activate a public stream, canvas, draft,
save, transport dispatch, or app workflow. Krita remains deferred.

## Root-owned registration requested

After reviewing the leaf source, root must make these **separate** edits. They
are intentionally absent from this candidate:

```toml
# core/crates/vw-capture/Cargo.toml [dependencies]
vw-stream = { path = "../vw-stream", version = "=0.1.0" }
vw-proto = { path = "../vw-proto", version = "=0.1.0" }

# Add to the existing windows features array:
"Win32_System_Diagnostics_ToolHelp"

[[bin]]
name = "vw-capture-stream-helper"
path = "src/bin/stream_helper.rs"
```

```rust
// core/crates/vw-capture/src/lib.rs
pub mod stream;
pub mod stream_wire;
pub mod stream_process;

// core/crates/vw-capture/src/windows/mod.rs
pub mod stream;
```

Do not substitute the one-shot helper or its `LockedHelper`: that type has no
stream path and its ordinary spawn has no pre-resume Job ownership. Root must
package the new helper and pass its exact trusted path/SHA-256 from a closed
runtime inventory to `PinnedStreamHelper::open`. A request, local file hash, or
helper reply cannot mint admission. Root also owns the authenticated
CONTROL/MEDIA/BLOB dispatch and caller checks before WGC launch and publication.

## Producer boundary

`StreamRequest` holds explicit owner action/process, UUID session, connection
epoch, source generation, the already-selected `PickerSource`, fixed physical
geometry revision, 15–60 target fps, and explicit endpoint/ring bytes. Validation
refuses missing consent, invalid or oversized geometry, zero epochs, a ring that
cannot hold eight originals, and a source that cannot fit the bounded tile grid.
No picker enumeration or source substitution occurs in the producer.

`StreamProducer::open(request, endpoint_budget)` owns one
`FrameRing::default_budgeted_for` (or the corresponding eight-frame budgeted
constructor for a smaller explicit ring limit), one `StreamPacer`, and exact
frame metadata. `poll` first obtains a native frame clock, reserves the ring
frame, then calls `StreamSource::read_into` on its already-admitted allocation.
WGC retains one capture session and the actual frame through readback; it checks
the cursor getter and current selected window/monitor and SDR output on each
read. The encoded MEDIA DTO and serialized transfer use `encode_tiles_budgeted`
and `to_transfer_bytes`; `published(sequence)` is called only after the full
pipe write. Next-poll feedback tells the pacer whether the endpoint admitted
that offer. A drop or repair forces a fresh keyframe.

`original(exact_identity)` pins only the requested retained frame and calls
`encode_original_budgeted`; wrong session, geometry, timestamp, or evicted frame
returns `Unavailable`. It never picks the latest frame. The caller owns
presentation tickets, provisional strokes, lossless swap, durable document
creation and source-close behavior. The producer makes none of those decisions.

`stream_wire` has closed Open/Poll/Original/Stop commands and
Ready/Tiles/Original/Idle/Refused/Stopped replies. It limits headers to 16 KiB,
payloads to 64 MiB, and retains charges for parsed headers, queued commands and
payloads. A no-allocation lexical census caps JSON at 512 tokens, 16 levels
and 1024 source bytes per string, and rejects raw control/DEL bytes. The
corresponding up-to-2.25-MiB workspace reservation includes eight possible
copies of the source text for decoded strings, temporary growth and error
formatting; it precedes Serde parsing and stays with the DTO. This closes the demonstrated 16,030-byte malformed
header that allocated 278,646 bytes under only 32,414 bytes of admission. The
new regression refuses that header without calling the allocating parser and
also checks that a normally shaped header cannot enter parsing one byte below
its calculated workspace charge. A separate test-only metering allocator,
isolated in `tests/stream_wire_allocation.rs`, checks cumulative successful
heap requests against the held charge for eight accepted escape/Unicode
variants, and checks bounded rejection paths,
including a long escaped/control-bearing string. These are written but unrun
until the shared build slot is granted. Protobuf tiles receive
a bounded wire preflight and an endpoint-pool parser/DTO reservation before
`FrameTiles::decode`; this uses the current `vw-proto` API. `validate_reply`
takes the same `SessionBudget` and checks sequence, epoch, generation, source geometry,
tile protobuf admission/format, and exact original identity/hash; these values
remain correlation data, not authorization. The host and helper each enforce
their local endpoint budget; root must bind both ends to the authenticated
session's agreed limit and reserve downstream display/transport copies. The host
gets its exact pool from `StreamProcess::budget()`; `Packet::belongs_to` can
confirm its charged inbound owner before adoption.

On Windows, `StreamProcess::open` retains no-follow executable and ancestor
handles plus trusted hash, spawns suspended without a shell, assigns the child
to one kill-on-close Job, then resumes its uniquely observed primary thread.
One bounded pipe worker and one in-flight exchange own charged queues. Cancel or
timeout closes the command sender, terminates the Job if needed, then waits for
actual process exit and IO join. An unresolved close keeps the process/slot/
helper in a retryable registry; `retry_unresolved_retirement()` attempts a new
bounded join. The helper has a separate self-owned memory/process cap and
watchdog. No real process or WGC capture was launched for this handoff.
The helper watchdog is disarmed while waiting for a command, and armed only for
Open/command processing and pipe publication; idle time is never charged to the
next command. WGC creates D3D on the adapter of the selected output using the
existing monitor-adapter selection helper, rather than a default GPU.

## Focused evidence and open validation

`stream_producer.rs` exercises consent/source/ring refusal, keyframe repair,
numbered frames, exact original retrieval, eviction and cancellation with a
synthetic source. `stream_wire.rs` exercises bounded/truncated/unknown headers,
pre-parse DTO admission, wrong-pool refusal, sequence/epoch/generation mismatch
and payload refusal using memory pipes. The local protobuf preflight cases
refuse duplicate/inflated fields and admit a fully populated 128-tile wire
grid, so the parser ceiling does not reject the valid tile-count maximum.
`rustfmt --edition 2024` parses and formats the seven new Rust leaves; staged
diff and secret scan are the only additional local checks. The tests have been
written but not compiled or run; no Windows Job/WGC, native codec, app, device,
provider, 15-fps performance or hardware gate result is claimed. Root's shared
build slot must first register dependencies/modules and compile the coherent
candidate, then run these tests and bounded Windows runtime fixtures.

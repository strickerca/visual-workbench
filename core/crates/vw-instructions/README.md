# Instruction and marker workflow adapters

This is source-only software for T2.04/T2.05, using BUILD_SPECIFICATION sections
4.2–4.4, 4.8 and 5's instruction-entry behavior. It is not a second model, journal,
recognizer, transport or package format. All persistent edits are ordinary
`vw-proto` transactions applied by `vw-ops` and persisted by `vw-store`.

## Canonical edits and references

`DocumentView::new(project, revision, document_id)` borrows the current model and
checks its full canonical state hash. `SourceBinding` carries project, document,
accepted host sequence and full visible-state hash; an optimistic edit at the
same host sequence changes the hash. Canonical snapshot serialization is admitted
against 64 MiB before hashing. Existing marker numbering is read literally.
Missing, duplicate or noncontiguous numbers require explicit reconciliation;
this library never rewrites imported or remotely accepted content on read.

`place_marker` creates the marker and its dedicated instruction in one transaction.
It appends the next number in accepted placement order, not UUID order or wall
clock order. Image/capture point markers snap to pixel centers and their box
edges snap to integers; PDF/SVG coordinates keep their document units. A PDF marker
requires an explicit layer page index; ambiguous page-less marker coordinates are
refused on placement/export. Placement
uses existing fractional ordering and appends to the chosen unlocked layer.
`delete_marker` deletes that instruction and object and renumbers later markers
in the same transaction. Their object/instruction IDs, text and appearance remain
stable. If a later marker is locked, the whole deletion is refused. Canonical undo
restores these ordinary operations; no special inverse state is stored here.

`set_instruction` preserves the complete supplied text and explicit entry method,
sets target roles independently of style/color, and rejects duplicate/cross-document
targets and conflicting links. Empty targets represent one optional global
instruction in this document's package. Existing live markers cannot be stranded
by retargeting/deleting only their instruction. Historical deleted-object references
remain canonical data: `detached_instruction_ids` reports them, export refuses
them, and only an explicit delete/update operation can change them. Ambiguous
legacy links need a future reconciliation UI; no content is silently selected,
trimmed, truncated or discarded.

An `EditPlan` contains an immutable ordinary transaction and its strict revision
precondition. `submit` is an **in-memory** HostSequencer integration/reference path:
an exact accepted retry returns the canonical original receipt, while a new stale
plan fails before mutation. Thus two marker plans from one revision cannot both
assign the same number. Refresh and explicitly replan the user's retained intent;
do not silently rebase numbering over accepted remote content. Concurrent/offline
UI draft reconciliation is not implemented by this crate.

Production FFI integration must hold its project mutation lock while checking
`plan.check_current(store.project(), store.revision())` and committing
`plan.transaction()` with `ProjectStore::commit`. An exact previously accepted
transaction must follow the store's exact-byte retry path before that stale guard.
Never apply a clone of the transaction without its workflow precondition and
then claim workflow invariants. The crate does not bypass storage durability or
authenticate a sender from the transaction's text/device field.

## Entry and focus lifecycle

`EntrySession` binds a fresh local session ID to one instruction, source revision,
focus generation and explicit `EntryMethod` (`pc_keyboard`, `phone_keyboard`,
`voice`, `handwriting`). It accepts bounded complete text snapshots with monotonic
callback sequences. Older snapshots are ignored, exact repeats are idempotent,
and changed bytes under one sequence are refused. Partial recognizer results
remain transient. Explicit `commit` returns an edit plan; cancellation closes the
session and clears its draft. Stale revision refusal retains the draft for an
explicit owner choice. Changed field/session tokens and callbacks after commit
or cancellation cannot mutate another instruction.

The platform must report actual provenance. These enums do not establish Android
on-device dictation availability, Win+H behavior, Android 14 handwriting support,
Samsung S Pen integration, permissions, recognizer quality or A24 acceptance.
No audio, handwriting samples, OS services, credentials or provider calls exist
here. No text result automatically sends an AI request.

`FocusEvent` is bounded application JSON, **not** an added wire protocol message.
It binds marker/instruction IDs to the exact source revision, authenticated peer,
fresh connection epoch and monotonic sequence. The transport adapter owns grants,
authentication, current epoch agreement and actual reliable/ephemeral routing.
Remote placement focus should be emitted after the accepted revision is known;
an optimistic pre-ack hash/host-sequence pair is not an accepted host revision.

`FocusFollower` keeps only one latest pending event. Future/not-yet-visible exact
state can resolve when OPS arrives; superseded revisions are ignored and unresolved
events expire after one second. Unknown identities, changed duplicate sequences,
foreign connections, malformed/oversized payloads and clock rollback are refused.
No events survive connection replacement. Local selection changes or disposal call
`invalidate`; disabling follow also invalidates pending/issued tickets. UI code
must call `is_current(ticket, view, monotonic_now)` after asynchronous dispatch.
This prevents a late focus result from overriding a new field, revision or epoch.
The ticket reports receive-to-ready time against 200 ms only; no code test here
measures the required phone-placement-to-PC-field latency. Focus never changes
the local camera or implies viewport Follow.

## Package compilation boundary

`InstructionExport::from_view` is a deterministic intermediate projection for
T2.08. It retains exact revision/IDs, literal authored text, roles, entry methods,
language and canonical update times; transformed marker coordinates include PDF
page identity. It includes every object's role independently of appearance,
including unlinked regions and Role::None context objects. Hidden/layer visibility
flags are explicit. Invalid links, dangling targets, role disagreement or duplicate
globals fail rather than producing an ambiguous package. Opaque captured EIDs
remain untrusted; their export set is sorted/deduplicated without changing model
data. Captured names/text are not promoted to authored instructions.

`to_json` bounds serialization before allocation. `prompt_fragment` uses literal
JSON strings inside Markdown code spans whose delimiter exceeds every contained
backtick run. `quote_untrusted` applies the same rule to captured names, text and
IDs: embedded newlines, headings and backticks remain quoted data. The required
untrusted-text notice and preserve-outside-change constraint are always included.
Role::None is explicitly context only. This is a package section, not a full
`prompt.md`/`vip-1` manifest; T2.08 owns image/crop generation, target coordinates,
semantic snapshot resolution, file hashes and package publication. Downstream
compilers must resolve geometry/assets against `source()` exactly, preserve the
full binding and account for region roles rather than infer intent from color.

## Bounds and verification status

Per-document limits are 16,384 objects, 512 markers, 2,048 instructions, 64 targets
per instruction, 32 KiB of UTF-8 text per instruction and 2 MiB of total instruction
text. Markers allow 64 opaque EIDs of 4 KiB each, with 2 MiB total EID bytes. Focus
JSON is limited to 4 KiB; export/prompt projections to 32 MiB. Rejections preserve
canonical content and never truncate. These are admission limits, not performance
measurements or a replacement for the application's scheduling/memory budget.

At source handoff the regression sources cover numbering, atomic delete/undo,
stale concurrent plans, durable checkpoint retry, all four entry methods, literal
text/role separation, locks, detached references, async entry/focus races, exact
revision fencing, expiry, malformed peers/payloads, transformed PDF coordinates
and injection-safe deterministic export. They have not been executed by this
agent. The parent owns formatting, license/build gates and serialized host/device
tests. Shared FFI, both instruction UIs, authenticated focus routing, recognition
adapters and the physical 200 ms/A24 checks remain integration/acceptance work.
G0/G1 hardware gates remain open; this source does not close T2.04 or T2.05.

Only already-pinned model/ops/proto/geom, serde, serde_json and thiserror are used.
The existing workspace glob includes the crate; no root manifest, lockfile,
protocol schema, canonical model, frozen mask/AI source or shared app was edited.

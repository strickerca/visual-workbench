# T3.11 carrier reconnect source checkpoint — 2026-10-19

This is a bounded source change, not a passed two-second carrier or Phase 3 gate.
The branch was reconciled with `origin/main` at
`6d505416963fb42009921bd3fb8639994e73752a` before this edit. Main already
contains the prior conflict-review store, native adapter, caller panels and
coordinator hooks; the parallel recovery-batch branch owns the bounded outbox
rebase changes. They were reused rather than duplicated.

Owned paths for this checkpoint:

- `core/crates/vw-ffi/src/session/carrier_switch.rs` — new bounded client
  selection and three deterministic source regression cases.
- `core/crates/vw-ffi/src/session/link.rs` — module declaration and one call-site
  replacement in `establish`'s client arm, plus imports. No connected-session,
  preview, media, input, authentication or image stream code changes.
- This handoff document.

The existing `routes` function still admits only one explicitly supplied
address per carrier, rejects non-loopback ADB and sorts tether, Wi-Fi, ADB. The
client now starts those candidates with 0/100/200 ms preference spacing and a
single 1,800 ms connect-and-Hello window. A slow or unreachable preferred
interface no longer consumes the full underlying 15-second dial before a viable
fallback is tried. Each candidate independently obtains current pinned TLS,
performs the normal carrier authentication and full Hello/HelloAck. Only the
first successful authenticated application session is returned; the other
attempts are aborted. An all-failed auth or protected-storage error remains
visible, and a stalled set returns Timeout for the existing bounded retry
driver. No alternate peer, weaker certificate check, identity, control grant,
durable OPS sender, outbox or draft transition was added.

The three new cases model a stalled preferred route with a ready authenticated
fallback, mixed authentication/transport refusal, and a completely stalled
attempt. Existing store tests in `core/crates/vw-store/tests/conflict_review.rs`
cover draft interruption/reopen, interruption after accepted edit before local
metadata, exact duplicate delivery, a foreign or stale checkpoint, a stale
never-sent draft, counter reservation and retained edits. The separate
recovery-batch branch has additional bounded outbox/rebase cases. These tests
are written but **unrun here**: no shared heavy build slot was granted. No
Windows/Android carrier, device, reconnect timing, Clippy or complete license
gate is claimed. Rustfmt, whitespace and staged secret checks are the cheap
checks for this checkpoint; their exact result belongs with the commit receipt.

For the integration owner: the link call site is already connected; no FFI,
schema, controller, root or binding hook is required for this helper. Existing
project `network_changed`/OPS, authenticated checkpoint installation, and
offline-review refresh continue to own draft/revision reconciliation. Integrate
the separately owned recovery-batch branch only after its review and shared
build slot. Then run focused `vw-ffi` carrier selection and link tests, the
existing conflict-review and recovery regressions, and actual tether/Wi-Fi/ADB
switches with synchronized timestamps. The 1,800 ms window covers connection
selection only; checkpoint transfer, app publication and the total ≤2-second
target still require physical measurement. Phone AI/provider sends require
separate authority and remain outside this implementation.

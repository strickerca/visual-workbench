# Transport integration

`PinnedTls` accepts an in-memory certificate/key and the exact trusted peer
certificate, device ID and DNS name. TLS 1.3 with ring authenticates both peers;
zero RTT and resumption are disabled. The test suite creates throwaway certificates
in memory. Production trust provisioning belongs to T1.06b. The `test-support`
feature exposes synthetic authentication only to the deterministic simulator.

Wrap a `TcpCarrier` or `QuicCarrier` in `CarrierIo`, then use
`SecureConnection::client/server` for the version/capability/connection handshake.
The session requires v1.2 connection, receipt and preview-lifecycle bindings. A refused Hello
returns an upgrade/identity message; unsupported application messages before
Hello are rejected. TCP sockets supplied through adb still require this TLS.
QUIC uses a stable bidirectional stream each for CONTROL, OPS, INPUT and MEDIA,
EPHEMERAL datagrams and bounded concurrent unidirectional BLOB streams.

Call `Session::enqueue_transaction` to commit local OPS to `ProjectStore` before
queueing. Backpressure and disconnect leave pending edits intact. After a fresh
connection, use `resend_pending` and `SyncReceiver::request`. The host uses
`HostSync::commit_persisted`, which returns its acknowledgement after SQLite
commit. Clients apply host-authenticated `SyncBatch` entries through the actual
operation engine, checking original receipts and each canonical hash. Unknown
bases receive a BLAKE3 checkpoint manifest; receive its bounded blob, then call
`install_checkpoint`. Only `acknowledge_pending` against that verified accepted
journal removes pending edits. Never interpret a receipt by itself as a rebase.

After the handshake call `SecureConnection::into_duplex`. Its independently owned
receive half continues processing while the bounded writer runs. At most one
write per QUIC channel is in flight, with separate stream locks and transport
priorities for CONTROL, INPUT and OPS. The driver keeps unsent work in the session
queues, so waiting MEDIA continues to coalesce. TCP keeps one framing writer:
an already-started TCP frame cannot be preempted, but the opposite direction is
independent and high-priority work wins the next frame. BLOB requests use priority zero for visible work;
bulk BLOB and MEDIA alternate when both are pending. Each channel has a byte
quota and an item limit. EPHEMERAL and MEDIA hold at most one unsent item per key.
MEDIA producers must provide an independently decodable keyframe initially and
after `KeyframeRequired`: replacing an unsent predictive delta invalidates its
reference chain. The old slot is removed and further deltas are refused until a
fresh keyframe arrives. A caller must also regenerate a keyframe after any failed
media enqueue. Frame IDs must increase within a capture; stale frames are ignored.
Lossless frozen images travel through reliable BLOB, not the lossy latest slot.

Ingress uses separate bounded queues and selects CONTROL, INPUT and OPS before
bulk delivery. Received MEDIA and EPHEMERAL coalesce per connection/capture or
gesture; stale IDs cannot replace a newer slot even after it has been consumed.
Replacing an unconsumed predictive frame or skipping its dependency discards
that chain and returns a recoverable `KeyframeRequired`; subsequent deltas remain
suppressed until a fresh keyframe. Reliable queue overflow fails the connection
instead of silently losing work or letting bulk FIFO slots block control reads.
Dropping the duplex receive handle closes the carrier and revokes INPUT; no
session lock is held over network I/O. Cancelling a partial frame write poisons
the connection so its remaining bytes cannot be mistaken for a later frame.

Consumed volatile identities retire from bounded LRU replay history instead of
exhausting the connection over many gestures/captures. Each channel has its own
retired sequence floor for unknown identities; still-tracked identities retain
their own watermarks so valid delayed updates to queued work are preserved.
Queued identities cannot be evicted to make room. Retired MEDIA identities must
restart with an independent keyframe. Ingress history belongs to the first
connection ID on that carrier; a different connection cannot poison its floors.
Application gesture completion needs the separate reliable v1.2 open/close
generation protocol: transport replay history alone cannot retire gestures whose
commits arrive on another channel before their previews.

`PinnedTls::new_authorized` preserves the pairing component's live revocation
policy in both halves. Per-frame policy reads run on Tokio blocking workers;
handshake WebPKI/signature checks remain mandatory. Production application calls
must supply that live trust policy rather than a stale static binding.

`BlobSender/BlobReceiver` enforce 64 KiB chunks and a 256 KiB outstanding window.
Resume from a verified local staging prefix; the receiver rehashes that prefix.
Publish original bytes only after `finish` verifies declared length and BLAKE3.
Retain the staging sink separately if a transfer disconnects. Duplicate chunks
must match already written bytes exactly. A final verified acknowledgement is
distinct from merely sending the last chunk.

On carrier failure the connection invalidates the session. The connection state
machine exposes discovery/handshake/connected/reconnecting/offline transitions;
the retry target is two seconds, and applications supply monotonic elapsed time.
Construct a fresh authenticated connection for a retry, clear rendered ephemeral
overlays, resync OPS, and require a new local user INPUT grant. Grants bind capture,
geometry and monotonic event sequence, expire after ten idle minutes and are
revoked on disconnect. Target, focus or geometry changes must explicitly revoke
the grant before platform input injection. Carrier deadlines are 15 seconds;
applications send periodic CONTROL pings while otherwise idle.

The carriers do not alter adb, firewall, Wi-Fi or tethering settings. Loopback
tests and the deterministic simulator are software evidence. They cannot establish
physical-link throughput, newest-frame rendering, reconnect latency or S23
acceptance; those remain hardware checks in the task evidence.

# GP1 nonblocking regular-file admission repair

Astra found a real Linux pathname race in79d7d7fc: a regular source can be
replaced by a FIFO after symlink_metadata but before blocking RDONLY open. The
joined caller then cannot observe cancellation until open returns, retaining its
input permit/charge. The parent reviewer reproduced that blocking syscall path.
Other public intake/preparation/cache/publication/rebuild/failed-owner source was
narrowly accepted, with final ABI/fourcaller checks still deferred.

The production correction adds OFlags::NONBLOCK to the existing Linux/Android
RDONLY|NOFOLLOW|CLOEXEC descriptor open. The actual descriptor's is_file check is
preserved: a replacement FIFO opens without waiting for a writer and is refused;
regular file reads retain normal behavior. Windows code is unchanged. This does
not promise general filesystem deadlines or hardware acceptance.

A cfg(test)-only exact-path/thread-local/one-shot interleaving replaces the real
selected file with a real FIFO immediately after successful pathname validation.
The regression calls the ACTUAL public open_giant_original boundary. It checks
Invalid refusal, unchanged shared memory, no work-directory creation, then
restores the synthetic regular file, admits BOTH input owners, closes them and
checks quota/directory retirement. No global counter is reset.

The parent runs this case by exact name in an isolated ignored child, checks that
one test actually executed, and enforces a five-second watchdog. It kills/waits
only its own child on timeout and removes only its owned synthetic tree after
actual process exit. Removing NONBLOCK would block this deterministic path until
the watchdog fails. This candidate ran the corrected path; it did not independently
repeat the reviewer's baseline experiment.

Focused command:

```sh
cargo test --locked -p vw-ffi --lib fifo_replacement -j2 -- --test-threads=1
```

Final result:1 parent passes,0 failures,1 private entrypoint ignored in the parent
and explicitly invoked/passed by that bounded parent. Rustfmt/source whitespace
checks pass. Prior85-native and10-ownership receipts remain bound to their earlier
source; no broad native/app/ABI/device/performance run was repeated. GP1 needs
narrow source re-review. The exact patch/log hashes are recorded in its JSON
receipt. The source-integration checkpoint dd18c238 separately composes accepted
offline hooks and Desktop/Air repairs without depending on this candidate.

The backend grant is committed separately at03ad446 for owner
01a15009-f394-70eb-9e10-e873087f1003, with exact frozen API/research references and
bounded source-pixel leaves. Model conversion/artifact/license admission and real
inference remain open; the .pt source hash is not a deployable encoder hash.

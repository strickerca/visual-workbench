# Coherent Windows release-file metadata — 2026-10-20

The Windows task replayed the release assembler on Python3.14 and reported
18 failures, eight errors and seven POSIX-only skips. Both ordinary input reads
and completed output writes failed when comparing pathname and open-handle
snapshots. This is parent-relayed Windows evidence; cloud did not execute that
baseline or inspect its raw log.

CPython3.14 uses two different meanings for Windows `st_ctime`: the
[pathname-stat implementation](https://github.com/python/cpython/blob/3.14/Modules/posixmodule.c)
copies creation time into that field, while the
[handle-stat implementation](https://github.com/python/cpython/blob/3.14/Python/fileutils.c)
retains native change time. Comparing them falsely reports an ordinary file as
mutated. Dropping change-time checks would weaken the actual mutation contract.

`tools/release/assemble.py` now obtains both Windows path and descriptor metadata
through the same native handle queries. `FileBasicInfo` supplies creation,
write and change clocks; `FileStandardInfo` supplies size, link count and type;
`FileIdInfo` supplies volume and the full128-bit file ID. The path query refuses
reparse points and resolved aliases, and closes its short shared metadata handle.
The existing retained read lease still denies writes/deletes. Every input,
output, namespace and completion-marker comparison keeps identity, size, write
time, change time and link count. Missing native fields fail closed. POSIX
continues to use its descriptor-relative stat and no-follow operations.

Fresh cloud validation runs48 assembler methods:46 pass and two new actual
Windows witnesses are skipped on Linux. Five new portable methods exercise
native field decoding, required queries, path-query lifetime, identity/type/
reparse/link checks and change-time-only mutation refusal. The two Windows
witnesses check real path/handle/close agreement and a same-length rewrite with
restored modification time. They must run on Windows; Linux cannot pass them.
Independent Astra source review and actual Windows replay are recorded separately.

For a sanitized Windows reproduction, run the exact pinned interpreter with
`-I -S -B tools/release/probe_windows_metadata.py --work <fresh-private-absolute-directory>`.
The parent directory must exist. The probe creates only a fixed-byte owned file,
retains that work and prints differing field names and comparison booleans. It
prints no host paths, timestamps, volume/file IDs, credentials or device data.
It also reports any native before/after-close disagreement rather than hiding it.

No MSI, installer, product assembly, GUI, provider or device operation is part of
this repair. See the [source-bound receipt](evidence/WINDOWS_RELEASE_METADATA_20261020.json).

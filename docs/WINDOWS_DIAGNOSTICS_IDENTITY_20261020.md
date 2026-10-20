# Windows diagnostics ownership repair — 2026-10-20

Base: `3bb8b7a6329d3c040dffbd885af3ec5b1b61a584`. The Windows lane reported
five diagnostics publication failures because JDK21 returned a null
`BasicFileAttributes.fileKey()`. No null-key, timestamp, size or content fallback
is admitted as file identity.

Windows now creates and writes the sibling through its own Win32 CREATE_NEW
handle. Its volume and full 128-bit file ID are captured from that actual handle,
which remains retained until cleanup really retires. The creation handle denies
competing writes while allowing replacement during preparation. Publication
opens a second no-follow handle, requires the original identity and exact size,
and denies rename/delete while the no-replace hard link is created. Both handles
together deny foreign writes and replacement through publication.

Cleanup checks the selected sibling through a DELETE-capable no-follow handle
and sets disposition on that same opened link. It never validates one pathname
and then deletes a newly opened replacement. A replaced, absent, reparse or
unidentifiable sibling retains pending ownership. Failed native closes retain
their actual handles; a scheduled-deletion retry retires those handles without
reopening or touching a later replacement. Existing foreign destinations and
foreign sibling bytes remain protected. NIO retains its strict non-null-key
checks. The chooser, cancellation/commit gate and UI owner remain unchanged.

The native layouts follow Microsoft's [FILE_ID_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_id_info)
and [FILE_DISPOSITION_INFO](https://learn.microsoft.com/en-us/windows/win32/api/winbase/ns-winbase-file_disposition_info)
contracts. Native identity bytes are never added to diagnostics or evidence.

The shared `bounded_owner_backed_receipt_import` wrapper now expects the unchanged
runner's 16 logical groups. Source inspection reconciles the owner-backed
admission/refusal, malformed decoder, epoch, borrow closure, joined observation,
producer/callback cancellation, palette import, NotTested observation and busy
closure groups. This is the runner's established literal-count convention, not
an automatically discovered test count. The actual runner executes successfully.

Focused cached Kotlin compilation and JUnit execute 22 methods on Linux:
11 existing publication tests, 10 synthetic native-call contract tests and the
shared wrapper. The publication body is a literal extraction from the actual
UI source; the full new helper and the shared wrapper and unchanged runner are
compiled. Unchanged shared dependencies use existing compiled classes. No UI
stub replaces the publication, and no Gradle/native build or binding generation
runs. Astra independently clears the final source. Three offline policy checks
pass. These checks do not execute Win32 APIs or compile the entire Compose app.

Windows replay must include `DiagnosticsPublicationTest`,
`WindowsDiagnosticOwnedFileTest` and shared `RemoteCompatibilityImportTest` in
the local lane's focused JVM runner. The existing real publication cases then
exercise Windows native creation, replacement refusal, hard-link publication,
cleanup and cancellation. Full managed validation remains available through
`build.ps1 test-apps` in the local lane's serialized build slot. Preserve any
failure trace; Windows success is still pending in this cloud receipt.

The [machine-readable receipt](evidence/WINDOWS_DIAGNOSTICS_IDENTITY_20261020.json)
binds source, the 16-group inventory and exact focused logs. Device, provider,
app-image, installer, physical and product acceptance are not claimed.

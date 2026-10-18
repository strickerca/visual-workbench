# Private verification archive prerequisite

Code `58508058555b51f0c5e8f73d597fd4b861729b0f` on cloud/verification-archive-privacy-20261018 changes only the root
archive exporter and its tests over reviewed main074b6c6. Pending native worker
8912cc39 and lineage8bf738ad remain uncomposed awaiting admission. This independent
prerequisite does not register persistence, FFI/UI or a manual owner-review setter.

The actual private SQLite copy now passes the existing bounded schema verifier
before any scrub. A DELETE trigger could otherwise relocate removed note values
to an unrelated setting that remains in the export. One injected-trigger test
proves export refuses before redirection and preserves the source. The private
copy deletes every `vw.verification-review.v1.*` row, including malformed records,
then final VACUUM removes former bytes. Source progress, unrelated settings,
canonical state/revision and original blobs remain intact. Later module admission
replaces the exact literal through VERIFICATION_REVIEW_PREFIX; its value is the
worker's frozen `vw.verification-review.v1.` namespace. No new namespace inferred.

All15 archive cases and11 existing local-view cases pass, including three new
privacy cases: full export/import/raw-byte removal with source retention, injected
schema redirection refusal, and post-scrub blob failure with no publication or
source loss. Strict all-target store Clippy passes; first attempts succeed.
Formatting, whitespace and source secret scans pass. No dependency/public ABI or
binding generation. Exact source/log receipts are in
`evidence/VERIFICATION_ARCHIVE_PRIVACY_20261018.json` and excerpts.

New root source still requires Astra review. This is known-namespace export
privacy, not arbitrary secret redaction or measurement/capture provenance. No
other granted files were edited. Full T5.01, image alignment and physical/device
acceptance stay open. Androidcd3c6437 and Desktop6eba1556 are fetched only pending
source verdicts; one coherent platform caller build/test batch follows admission.
No private capture, provider/credential, signing/deployment, SDK/device or old-
worktree operation occurred.

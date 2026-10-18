# Document-copy CORE-006 interface handoff

Source batch is separate from frozen integration candidate 070758a. Root owns
protocol/shared/native mapping and application publication admission; document
lane owns exporter carriers, isolated final-byte proof and typed result metadata.

`vw_document_protocol::Request.export_identity: Option<CopyIdentity>` is a top-level
field. `CopyIdentity` uses deny-unknown-fields snake-case JSON:

```json
{"schema_version":1,"host_seq":42,"state_hash":"<64 lowercase hex>","source_asset":"<64 lowercase hex>","capture_session":"<canonical lowercase UUID, optional>","frame_id":0}
```

`capture_session` and `frame_id` are both absent or both present. Unsigned 64-bit
values must retain their exact value. PDF/SVG source descriptors have no capture
context, so both are absent. `source_asset` equals the immutable input BLAKE3 in
JobBinding. `state_hash` is the complete revision hash. The file carrier must also
include `revision = r{host_seq}-{first 8 state_hash characters}`. `CopyIdentity::revision()`
provides this checked label. Neither project paths nor passwords enter provenance.

All four protocol copy operations (PdfCopy including Original, SvgCopy,
PdfRasterRedaction, PdfPcRedaction) require identity at application worker admission.
Info/Page/Tile/Text/Selection reject identity. The bounded protocol parser still
accepts absent identity for existing low-level fixtures; that is not publication
admission. Isolated copy dispatch must call `require_copy_identity()` before export.
Do not treat a legacy unbound copy or optional missing metadata as publishable.

Every successful copy result metadata variant adds `export_identity: CopyIdentity`
equal to the requested value. Isolated exporter must reopen/reinspect the actual
final bytes and verify the embedded identity including derived revision before
returning success. Host typed metadata validation compares the entire echoed
identity with request identity. Test missing/mutated/stripped carriers, another
source/revision, both unsigned maxima, all copy modes, fidelity and final-byte
hash/length. A sidecar or unverified echo alone cannot authorize publication.

Original immutable source remains unchanged. If an Original copy cannot preserve
its feature/fidelity promise while adding metadata, explicitly return Unsupported;
never silently return unbound bytes or switch to another mode. Preserve redaction
verification and cancellation/retirement behavior around final-byte inspection.

Shared names are `DocumentExportIdentity`, `DocumentRequest.exportIdentity`,
`DocumentMetadata.Copy.exportIdentity` and `DocumentExport.exportIdentity`.
DocumentInteraction constructs from its full revision/source snapshot and requires
exact echo. The actual application capability remains false until the exporter and
native result validation evidence land; this handoff alone does not enable export.
No UniFFI signature change or unrelated binding regeneration is required.

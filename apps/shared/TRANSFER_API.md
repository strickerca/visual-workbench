# File transfer API

`WorkbenchStreamingProject.exportFile` keeps its original signature and
`CoreFailure` mapping. It now accepts the explicit JPEG and WebP format variants
alongside PNG. `WorkbenchFileTransfers.exportTransfer` accepts the same options
and returns the same immutable file receipt, with typed `TransferFailure` errors.
`preflightFile` distinguishes per-side format dimensions, the 50 MP pixel limit,
memory, metadata/profile, alpha, depth, encoded size and scratch admission. It
verifies the original and its metadata without publishing an output. Marked
geometry and result-asset admission are repeated during the authoritative export;
`requiresRenderValidation` records this remaining preflight boundary.

PNG uses source-resolution strips. JPEG/WebP use the existing buffered encoders
only after retained source, full raster, color-profile and codec workspaces fit
the caller budget (maximum 256 MiB). No format silently resizes, reduces depth,
discards alpha or converts color. The caller selects quality, matte, depth-reduction
permission and sRGB conversion explicitly. Buffered codec/render calls are
synchronous and bounded; cancellation is checked before/after them and during
file writes, rather than claiming interruption inside the codec.

Outputs are staged in the exact private destination directory and published with
no-clobber semantics after flush and a final cancellation check. Partial files are
owned by the call and removed on failure. Once publication succeeds, a cancelled
caller cannot remove the accepted output. The receipt binds the file hash, encoded
size, embedded revision metadata and the core snapshot used by the export.
`estimatedPeakBytes` is a conservative reservation, not measured process RSS.

`attachFile` accepts image originals or ordinary MP4 assets, bounded to 64 MiB per
asset by the current carrier. It streams a private copy and hashes it, validates
the image through the shared raster pipeline or the MP4 top-level container
envelope, installs the immutable blob, then atomically commits one `AddAsset`
transaction. Exact transaction retries do not add history twice. Cancellation
before the transaction can leave an unreferenced immutable blob; accepted history
never points to a missing original. Attachment creates no document or timeline.
The MP4 check requires a supported `ftyp` brand plus bounded `moov` and `mdat`
boxes. It does not decode tracks or establish playback validity. The box roles
follow the [W3C ISO BMFF description](https://www.w3.org/TR/mse-byte-stream-format-isobmff/).

The shared cancellation adapter joins native work before allowing callers to
dispose their private inputs/scratch. Root integration adds `WorkbenchFileTransfers`
to `NativeProject` and forwards its three methods directly to `preflightFileNative`,
`exportTransferNative` and `attachFileNative`, preserving the existing closed guard.
Do not wrap these helpers in another cancellable dispatcher boundary.

Reverse acquisition of a phone-originated ordinary asset is integrated in the
session worker and passed synthetic native loopback tests before host acceptance.
These APIs and fixtures do not establish physical MP4 cross-device,
clipboard/share-destination, hardware, offline share-sheet or performance acceptance.

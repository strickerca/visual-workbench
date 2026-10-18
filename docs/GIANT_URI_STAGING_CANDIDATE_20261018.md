# Giant URI staging source candidate

The root native URI path now reserves the existing two-input/4MiB admission
before filesystem staging, Java holder allocation or ContentResolver open. It
never uses a raw FD or opens/spools foreign bytes before admission. No additional
input counter, resource pool, producer approval or completed-directory factory
was added. Native copies64KiB at a time, checks cancellation/deadline and encoded
size/disk/free-space bounds incrementally, flushes/syncs/rewinds its exact File,
and transfers that File, TempDir and same admission into PickedOriginal. Existing
prepare/build/publish/rebuild owners remain authoritative; originals are hashed
and inspected through their normal preparation path.

The fixed Android runtime only creates a holder initially. Native obtains strong
global holder/buffer references before opening the resolver, then invokes its
fixed open/read/close methods. The holder permits one open attempt, serializes
read/close and keeps the exact InputStream after close throws. No external
callback or caller-supplied retirement assertion can release the native input.
JNI errors are cleared without logging exception text or private URI data.

Actual reader close precedes staging cleanup/admission release. A failed close
retains the exact strong Java owner, bounded buffer, native file/directory and
same permit/charge in a bounded-by-input-admission quarantine. Explicit
retry_giant_uri_retirement retries actual close outside the registry mutex.
Unknown reader retirement never becomes success on timeout/cancellation. Existing
OwnedDirectory separately retains failed disk cleanup for process life; successful
reader retry is not evidence that an old directory quarantine recovered.

The new native `GiantUriOptions`/open_giant_content_uri boundary supports only the
compiled fixed Android resolver. Other platforms return Unsupported. The work
parent must already exist and pass existing native directory admission. Content
URIs are bounded/request-only; only Android ContentResolver's content scheme is
accepted. No persistent URI permission or device/capture grant is taken.

Shared `openAndroidGiantOriginal(context, options)` installs the fixed runtime
with the application ContentResolver and uses the existing acquireGiantImport /
settledStream ownership/undelivered result path. Cancellation joins actual native
read/close; an uninterruptible OS read may take longer than the requested deadline
and keeps its worker/input charged throughout. R8 keeps exactly the fixed runtime
and holder entry names. retryAndroidGiantUriRetirement exposes actual retry.
The native thin facade still requires the final generated ABI to compile/run.

Executed cheap checks: Kotlin2.4.20 explicit API/warnings denied compilation of
the actual common URI options, production InputStream holder and Android bridge
against installed SDK37.0;7 actual JVM holder tests; javap verification of all
five fixed JNI methods/descriptors; Rust syntax/format and whitespace checks.
Receipt: docs/evidence/GIANT_URI_OWNER_20261018.json. No generated/native/Android
replacement class was used for the holder tests. Cached predecessor shared and
bindings jars supplied unchanged project/error types only. Temporary class outputs
were disposed. The first probe failed before compilation because its SDK path
used android-37 instead of the installed android-37.0; failure receipts remain.

Six native production-algorithm tests are authored, UNRUN: admission before
filesystem/source open, pre-open cancellation, cancellation after actual read,
failed reader-close retention/quota/retry, byte/disk/read/count failures, and
same-admission byte-preserving transfer to the actual PickedOriginal. Native
Android JNI type compilation, final ABI, full shared/apps and actual device URI
journeys remain deferred. Suggested final commands include:

```text
cargo test --locked -p vw-ffi --lib giant_images::producer::uri_staging:: -- --test-threads=1
cargo check --locked -p vw-ffi --target aarch64-linux-android
python3 tools/tests/check_giant_uri_owner.py --tools-root /workspace/.vw-tools --receipt-dir /tmp/vw-giant-uri-final-fresh
```

Use the actual parent-approved build/license/toolchain entrypoint and one heavy
slot for native/target checks. The focused Python command requires a fresh receipt
directory and the tested setup environment; it does not generate bindings.

The Android picker/controller/app callsite is still missing implementation and
awaits explicit platform ownership handback. No giant URI import feature or
device gate is marked complete. Main/capture verification callbacks, V's archive
BLOB-prefix portability repair, and actual giant tonal/region composition remain
independent root code work. The current image_frames tile reader still fully
decodes sources under50MP; it must not be labelled a giant-region implementation.

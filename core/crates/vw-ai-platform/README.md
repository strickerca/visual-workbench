# Protected image-provider credentials and Android trust bootstrap

T2.02 additive source candidate, researched 2026-10-03. This crate implements the
reviewed `vw-ai-provider::SecretProvider`; it does not change the paid-once ledger,
HTTP transport, provider configuration, model or image proof. No key was accessed,
no provider call was issued, and none of these test sources has been executed.

## Storage and ownership

Windows uses the one current-user Generic Credential target
`VisualWorkbench/OpenAI/APIKey/v1`. `CRED_PERSIST_LOCAL_MACHINE` preserves it for
subsequent logons by this user on this machine; it does not make it a credential
for other users or enable roaming. Save/remove are explicit settings mutations.
Reads validate the OS record before copying at most 2560 visible ASCII bytes into
`Zeroizing<Vec<u8>>`; the returned OS blob is wiped before `CredFree`. There is no
enumeration, caller-selected target, environment variable, command-line argument,
project-file or plaintext-string fallback. Windows tests use synthetic in-memory
records only and never invoke Credential Manager.

Android owns one AES-256-GCM key alias `visual-workbench.openai.api-key.v1` and an
encrypted, versioned envelope under `Application.noBackupFilesDir` in the fixed
`vw-provider-credentials` directory. Directory/file modes are owner-only. AAD
binds the format version; random IVs and authentication tags bind ciphertext.
The bounded envelope contains at most 4096 API-key bytes. Existing ciphertext with
missing/unavailable wrapping material fails closed: an explicit Remove is needed
before replacement-key creation. Path/type checks reject symlink children. A file
lock serializes settings and native reads; AtomicFile publication, file/directory
fsync and decrypted readback precede a successful Save response. Failed publication
can leave an uncertain mutation; errors do not promise rollback. Remove checks
that its exact encrypted children were deleted, then deletes the wrapping alias.

`AndroidProviderKeyStore.save` consumes/wipes its input and gives the worker an
independent bounded copy. Settings use at most two workers with no queue and a
five-second caller deadline. Cancellation before publication prevents that write;
a caller cancelling during uninterruptible Keystore I/O returns while the worker
retains its slot and buffers until it actually exits. A timeout is Busy, and the UI
must recheck status after an uncertain save/remove. The provider's existing two
native-worker cap owns blocking JNI reads; a separate settings pool never owns a
paid request. Each native read returns a fresh Java byte array; Rust copies it into
zeroizing buffers and clears that exact Java array before returning. There is no
shared transient array that another read could clear prematurely.

Native startup retains one process-lived Application, class loader, JVM and fixed
production store. Repeated initialization must supply those same Java objects.
It never retains an Activity, never replaces `JNI_OnLoad`, and never creates an
extra native library. Actual verifier classes must load before `AndroidCredentials`
and `PlatformTrust` are ready. Failed bootstrap returns only a fixed unavailable
status; Java exceptions are cleared without printing them. The package bootstrap
does not read credentials. The settings/provider owner must explicitly check
`is_configured` and construct `PlatformTrust::system()` before offering Send or
consuming a paid permit. The provider still obtains a fresh protected read after
its exact one-attempt admission, so key availability can change between checks.

The Android password dialog offers explicit Save/Remove, never reads the saved
value into its field, disables saved state/autofill/keyboard learning requests,
uses a secure dialog window, and clears input on dismiss/background/disposal. It
does not authorize spend or send a request. Clear/wipe is best effort: keyboard,
Android, Compose/other UI immutable Strings and HTTP/header/TLS implementations
can retain copies that this layer cannot guarantee to zeroize. No full-memory
zeroization claim is made.

## Exact integration (central owner only)

1. Add `vw-ai-platform` to the Rust workspace and an exact `=0.1.0` path dependency
   from the final FFI library. Preserve all existing JNI/native packaging. Android
   FFI needs `jni = { version = "=0.22.4", default-features = false }` for the small
   forwarding signature below. This crate uses the same approved JNI pin and the
   existing `windows-sys = "=0.61.2"` / `zeroize = "=1.9.0"` pins. No broad upgrade.
2. Add the forwarding export to the existing `vw_core` library (not this rlib):

   ```rust,ignore
   #[cfg(target_os = "android")]
   #[unsafe(no_mangle)]
   pub extern "system" fn Java_com_visualworkbench_shared_AndroidProviderRuntime_initializeNative<'local>(
       env: jni::EnvUnowned<'local>,
       class: jni::objects::JClass<'local>,
       application: jni::objects::JObject<'local>,
       store: jni::objects::JObject<'local>,
   ) -> jni::sys::jint {
       vw_ai_platform::android::initialize_native(env, class, application, store)
   }
   ```

   The JVM supplies these typed local references. `EnvUnowned::with_env` is the
   first JNI operation; its outcome catches unwinding at the native boundary.
3. In the central settings repositories, add the official verifier Maven archive
   as **exclusive content for group `org.rustls` only**, then add
   `implementation("org.rustls:rustls-platform-verifier:0.2.0")` to Android shared
   source dependencies. The restricted repository URL is
   `https://github.com/rustls/rustls-platform-verifier/raw/maven-archive/android-release-support/maven/`.
   Resolve the version selected by `rustls-platform-verifier-android` paired with
   Rust verifier 0.7.1; reject a mismatch. Pin the fetched POM/AAR checksum in central
   verification metadata and retain the upstream notices before central builds.
   The POM was inspected; the AAR bytes/hash/native inventory were **not** fetched
   or verified in this source-only task. Its minimal POM has no license block, so
   an automatic Maven-license guess is insufficient.
4. Preserve these JNI names through R8, plus the verifier AAR's own consumer rules:

   ```proguard
   -keep class com.visualworkbench.shared.AndroidProviderRuntime { *; }
   -keep class com.visualworkbench.shared.AndroidProviderKeyStore {
       public byte[] loadForNative(long);
       public void clearForNative(byte[]);
       public boolean isProductionForNative(android.app.Application);
   }
   -keep class org.rustls.platformverifier.CertificateVerifier { *; }
   -keep class org.rustls.platformverifier.VerificationResult { *; }
   ```

5. After the existing core loader loads `vw_core`, call
   `AndroidProviderRuntime.initialize(application)` once from the Application owner
   and retain/use its fixed store. Catch `ProviderCredentialException` to disable
   provider readiness visibly. Do not initialize from an Activity, request worker
   or project. The existing manifest already disables backup; retain that setting.
   `noBackupFilesDir` independently excludes the new credential directory. Do not
   put the wrapping key, encrypted file or API key into any project/archive/export.
6. Wire an explicit settings action to
   `ProviderKeyDialog(store, activity.lifecycle, onDismiss)`. Native provider setup
   uses `Arc::new(AndroidCredentials::ready()?)`, or `WindowsCredentials` on the
   Windows bounded settings/provider worker, alongside `PlatformTrust::system()`.
   The dialog does not itself add a provider/project/Send API to existing facades.

## Source regression coverage and open acceptance

Five Rust unit sources cover byte limits/header injection, cancellation/deadline,
synthetic Windows record copying, invalid record metadata and the fixed target.
Eight Android instrumentation sources use unique **synthetic fixture-only** aliases
and no-backup directories, refuse those fixtures at native production bootstrap,
and check encrypted roundtrip/remove, separate read-buffer ownership, invalid
input wiping, ciphertext authentication failure, missing wrapping-key recovery,
malformed envelope bounds, symlink preservation and cancellation while locked.
Fixture cleanup waits for actual worker retirement, then removes only its exact
files/key alias. These tests do not read or mutate a user's production credential.

Unverified: Rust/Kotlin/JNI compilation, resolved AAR/DEX/R8 packaging, real Android
and Windows protected-storage behavior/restarts, bootstrap idempotence on device,
OS TLS validation and custom/enterprise root behavior, settings UI/accessibility,
crash/fault injection, process-death cleanup, FFI/Send wiring and live GPT Image
acceptance/cost. S23/physical acceptance and owner API/spend approval remain open.
Do not infer any of those from this standalone source adapter.

## Primary provenance, checked 2026-10-03

- JNI 0.22.4 stable API/license metadata: <https://docs.rs/jni/0.22.4/jni/>;
  <https://docs.rs/crate/jni/0.22.4/source/Cargo.toml> (MIT OR Apache-2.0).
  `EnvUnowned`, `Global`, `JValue`, `jni_sig!` and array methods were checked against
  the published 0.22.4 API. JNI is a binding, not a replacement JVM or TLS stack.
  Transitive jni-macros/jni-sys and their notices still pass central license gates.
- Verifier Android Runtime/init source:
  <https://docs.rs/rustls-platform-verifier/latest/src/rustls_platform_verifier/android.rs.html>
  and official setup <https://github.com/rustls/rustls-platform-verifier#android>.
  Rust 0.7.1 uses first-wins runtime initialization; this adapter provides one
  Application owner and rejects a foreign second owner. Upstream MIT OR Apache-2.0
  applies subject to resolved artifact/notice verification.
- Exact Android 0.2.0 POM:
  <https://raw.githubusercontent.com/rustls/rustls-platform-verifier/maven-archive/android-release-support/maven/org/rustls/rustls-platform-verifier/0.2.0/rustls-platform-verifier-0.2.0.pom>.
- Win32 API and persistence/blob bounds:
  <https://learn.microsoft.com/en-us/windows/win32/api/wincred/nf-wincred-credreadw>,
  <https://learn.microsoft.com/en-us/windows/win32/api/wincred/ns-wincred-credentialw>.
  Exact cached windows-sys 0.61.2 declarations were inspected (MIT OR Apache-2.0).
- Android Keystore/AES parameters:
  <https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec>;
  backup exclusion <https://developer.android.com/identity/data/autobackup>;
  atomic files <https://developer.android.com/reference/android/util/AtomicFile>.

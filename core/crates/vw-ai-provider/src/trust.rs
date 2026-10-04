use crate::{Error, Result};
use rustls_platform_verifier::BuilderVerifierExt;
use std::sync::Arc;

/// Construct before showing Send as ready. Contains OS-trusted certificate
/// verification with explicit ring, never caller-selected roots or verifiers.
pub struct PlatformTrust(pub(crate) rustls::ClientConfig);
impl PlatformTrust {
    pub fn system() -> Result<Self> {
        #[cfg(target_os = "android")]
        if !ANDROID_INITIALIZED.load(std::sync::atomic::Ordering::Acquire) {
            return Err(Error::TrustUnavailable);
        }
        let config = rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_protocol_versions(&[&rustls::version::TLS13])
        .map_err(|_| Error::TrustUnavailable)?
        .with_platform_verifier()
        .map_err(|_| Error::TrustUnavailable)?
        .with_no_client_auth();
        Ok(Self(config))
    }
}

#[cfg(target_os = "android")]
pub use rustls_platform_verifier::android::Runtime as AndroidRuntime;
#[cfg(target_os = "android")]
static ANDROID_INITIALIZED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Application startup only, after bundling the verifier's matching Android
/// component and obtaining real JVM/context/class-loader global references.
/// The supplied runtime is process-lived; it must never reference an Activity.
/// Verifies that the actual application class loader can load the matching
/// verifier classes before enabling Send. Live certificate acceptance remains
/// checked by TLS on every connection and requires separate device validation.
#[cfg(target_os = "android")]
pub fn initialize_android(runtime: &'static dyn AndroidRuntime) -> Result<()> {
    runtime
        .java_vm()
        .attach_current_thread_for_scope(|env| {
            let loaded = env
                .new_string("org.rustls.platformverifier.CertificateVerifier")
                .and_then(|name| runtime.class_loader().load_class(env, name))
                .and_then(|_| env.new_string("org.rustls.platformverifier.VerificationResult"))
                .and_then(|name| runtime.class_loader().load_class(env, name))
                .map(|_| ());
            if loaded.is_err() {
                env.exception_clear();
            }
            loaded
        })
        .map_err(|_| Error::TrustUnavailable)?;
    rustls_platform_verifier::android::init_with_runtime(runtime);
    ANDROID_INITIALIZED.store(true, std::sync::atomic::Ordering::Release);
    Ok(())
}

# JNI names called by the pinned native provider/runtime and platform verifier.
-keep class com.visualworkbench.shared.AndroidProviderRuntime { *; }
-keep class com.visualworkbench.shared.AndroidProviderKeyStore {
    public byte[] loadForNative(long);
    public void clearForNative(byte[]);
    public boolean isProductionForNative(android.app.Application);
}
-keep class org.rustls.platformverifier.CertificateVerifier { *; }
-keep class org.rustls.platformverifier.VerificationResult { *; }
-keep class org.rustls.platformverifier.StatusCode { *; }

# JNI names called by the pinned native provider/runtime and platform verifier.
-keep class com.visualworkbench.shared.AndroidProviderRuntime { *; }
-keep class com.visualworkbench.shared.AndroidProviderKeyStore {
    public byte[] loadForNative(long);
    public byte[] loadForNative(int,long);
    public void clearForNative(byte[]);
    public boolean isProductionForNative(android.app.Application);
}
-keep class org.rustls.platformverifier.CertificateVerifier { *; }
-keep class org.rustls.platformverifier.VerificationResult { *; }
-keep class org.rustls.platformverifier.StatusCode { *; }

# Fixed private runner and typed output reached through the native JNI bridge.
-keep class com.visualworkbench.shared.giant.AndroidGiantRuntime { *; }
-keep class com.visualworkbench.shared.giant.AndroidTileBuild { *; }

# Fixed URI holder is pinned before native opens the resolver; methods are JNI-only.
-keep class com.visualworkbench.shared.giant.AndroidGiantUriRuntime { *; }
-keep class com.visualworkbench.shared.giant.AndroidGiantUriReader { *; }

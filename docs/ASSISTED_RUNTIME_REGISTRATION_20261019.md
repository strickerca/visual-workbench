# Standalone assisted-selection runtime registration

The accepted Android backend lives in shared/androidMain, so the three exact
LiteRT2.3.0 API/JNI/GPU dependencies are now registered there with strict version
constraints. Android debug/release runtime and shared locks are generated from
the real graphs. This makes the backend's imported API available to the eventual
coherent Android build; it is not model or execution admission.

The official quickstart uses these standalone runtime coordinates:
https://developers.google.com/edge/litert/android. The exact three AAR bytes
independently match model-runtime-candidate.json. All three contain identical
13,575-byte LICENSE and1,960,672-byte THIRD_PARTY_NOTICE.txt files, retained once
in third_party/notices and indexed by exact hash/source archive in LICENSES.
The three arm64 native child identities also match the source owner's inventory.
The full aggregate notice covers source/build/runtime terms; individual native
component scope is still a release/runtime admission task, not silently declared
Apache-only from the Maven POM or from the act of copying notices.

The API's POM also pulls com.google.android.play:ai-delivery:0.1.1-alpha01 under
Play Core terms. The existing offline local-model backend never uses AI packs.
All three dependency edges exclude that module, removing its Play services and
asset-delivery chain without a license-policy exception. Independent classfile
inspection of all73 API classes finds Play references only in the four
AiPackModelProvider implementation classes. A conservative closure from the
backend's five actual LiteRT imports reaches36 API classes and no deployment or
Play references. Consumer rules retain reflection-annotated members rather than
the unused deployment provider. This is static evidence; actual D8/R8/APK and
device linking remain unrun. No missing-class warning suppression is added.

Executed checks under the one-build lock/max2 workers: real shared dependency
resolution, scoped Android debug and release runtime lock generation, complete
repository checkDependencyLicenses application census, and check_setup.py
third-party. The Maven census passes with264 dependency identities across all
12 required runtime configurations. Both Android runtime graphs contain all
three LiteRT2.3.0 coordinates and no Play delivery/services chain. Existing
Gradle deprecations/metrics warning remain; no Kotlin/native/app compilation,
binding generation, model conversion or inference was run in this batch.
An initial broad Android lock resolution was retained in /tmp, then its unrelated
tool/test lock expansion was replaced by scoped runtime-only generation.

Actual transitive version movements include AndroidX Lifecycle2.10.0,
SavedState1.4.0, Guava33.5.0-android and its annotation/failure-access dependencies.
Shared compile/device-test metadata contains its real resolved variants; the
Desktop source graph is unchanged. Final coherent Android tests/lint/package
must bind these updated runtime identities. Text source/artifact/graph/log hashes
are in evidence/ASSISTED_RUNTIME_REGISTRATION_20261019.json.

The Android owner may now consume the existing public backend factory and safe
empty catalog/currentSource interaction as granted. Real converted encoder and
decoder artifacts, reference parity, source/native/APK licenses and working-set
review are still delivery tasks. Keep pins null and catalog empty until the
exact detached admissions exist. Artifact files/checkpoint/private paths are
never guessed or replaced by synthetic model evidence.

# Giant composed viewport delivery

Parent may continue existing G owner01a1500a-13fe-77e4-835e-201034a9901c after
the reviewed native region handback. Root has now composed E47f76bf,
G08332934+b75cea58 and all three portable hooks, retaining URI intake/producer
exports and the explicit color argument. The composed Linux native library check
passes; G's8 raster/4 native/1 Kotlin regression methods remain unrun. This native
capability is not yet selected by an actual viewport owner.

G additionally receives exclusive ownership of these formerly root-owned leaves
for this narrow next task; root will not edit them concurrently:

- apps/shared/src/commonMain/kotlin/com/visualworkbench/shared/GiantViewportSession.kt
- apps/shared/src/jvmTest/kotlin/com/visualworkbench/shared/GiantViewportSessionTest.kt
- apps/desktop/src/main/kotlin/com/visualworkbench/desktop/GiantCanvasHost.kt
- NEW dedicated shared/Desktop mode/ownership tests

Keep NativeGiantImages, native/raster roots and registration, Cargo/Gradle,
controllers and both platform canvas roots excluded. Desktop EditorController/
EditorCanvas remain R-owned; Android GiantAndroidCanvas/CanvasSurface and its
controller remain with the Android owner. Send this exact small bridge to both
through the parent as soon as its source signature is available.

Add an explicit local content mode, preserving all existing positional budget
arguments by appending defaulted construction parameters:

```kotlin
public enum class GiantTileContent { Source, Composed }
// Append to GiantViewportSession after maxRetainedBytes:
content: GiantTileContent = GiantTileContent.Source,
assumeUntaggedSrgb: Boolean = false,
// Append the same options to the Desktop factory.
// Append an explicit content field to GiantVisibleFrame, default Source for
// old fake/source construction. A visible frame labels what was actually read.
```

Mode is immutable for this session. Source calls image.tile; Composed requires
the actual GiantEditingImage capability and calls editingTile(address, foreign,
assumeUntaggedSrgb). Missing capability refuses before renderer admission; it must
never fall back to raw cache pixels while labeling them composed. Preserve the
prefetched native publication/source owner, actual foreign reservation, final
pixel/identity/extents checks, cancellation join, exact failed retirement and
renderer/surface ownership. No second cache, project, resource pool or lifetime.

Include content mode in visible-frame admission and the actual Desktop matching
guard so stale/raw frames cannot satisfy a composed draw. Full binding/attachment
epoch/source/viewport-generation checks remain required. A mode or committed
revision change retires/replaces the existing viewport through its established
owner; do not clear a charged renderer list or swap a handle as retirement.
This is app-local metadata, not a protocol or shared permission change.

Composed data uses native original D origin and composes before reductions only
at supported levels0–2. Higher levels remain an explicit limitation; do not call
the old full-decode editingView or lossy/downsampled cache and infer canonical
tones. Forward false color consent unchanged. True requires the platform's
actual user-approved untagged-sRGB choice for this source; no inferred consent
from cache existence, initial import metadata or a previous project.

Return exact constructor/visible-mode hooks for R and Android to select Composed
when canonical tones/annotations are baked and suppress only those draws. Guides,
selection/handles and other unbaked overlays still draw. Geometry previews must
not display a moved vector over its old baked position or hide the entire
background. These actual controller/canvas changes remain their owners' work,
and source-mode behavior must stay byte/pixel compatible.

Focused tests must cover actual optional capability dispatch, consent forwarding,
source/composed mismatch refusal, stale revision/mode replacement, and renderer
retirement/cancellation with charged bytes. No broad bindings/Gradle/native/APK
or physical/performance run; full end-to-end evidence remains batched. Return
exact source/ABI/runtime limitations and reviewed publication hashes. No device,
provider, credential, signing, deployment or worktree cleanup action is implied.

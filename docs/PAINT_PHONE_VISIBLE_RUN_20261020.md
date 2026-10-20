# Visible phone-to-Paint run

Run `tools/remote-edit/paint_phone.ps1` in a fresh PowerShell 7.2+ process on
Windows. This opens a generated white BMP in a newly created, retained Paint
process, uses the existing real S23 remote session to send its synthetic short
pen stroke, and checks four native canvas-only WGC images. It does not open the
normal Workbench UI: that editor deliberately excludes its own window from
capture and has no retained blank-editor test entrypoint.

Reuse the admitted APKs, Desktop classpath and built native helpers. Regenerate
the source map/artifact manifest with the new caller sources; preserve the real
build receipt and artifact hashes. No APK, bindings or Rust rebuild is required
by this script-only change. Paint must match the existing native installed-Paint
policy; no package, permission or tool-selection fallback is implemented.

First compile the native owner in a separate fresh process, without launching UI:

```powershell
pwsh -NoProfile -Command "Add-Type -Path '.\tools\remote-edit\paint_phone_native.cs'; 'Native owner compiled'"
```

Then run from the repository root, substituting the existing admitted paths,
hashes and explicitly selected USB route. Use the actual Python executable,
rather than a WindowsApps execution alias:

```powershell
pwsh -NoProfile -File .\tools\remote-edit\paint_phone.ps1 `
  -Execute -OwnerReady -PaintPath $paintPath -PaintSha256 $paintSha256 `
  -ProbeRunnerPath $probeRunnerPath -ProbeRunnerSha256 $probeRunnerSha256 `
  -ProbePath $probePath -ProbeSha256 $probeSha256 `
  -FrameRunnerPath $frameRunnerPath -FrameRunnerSha256 $frameRunnerSha256 `
  -CaptureHelperPath $captureHelperPath -CaptureHelperSha256 $captureHelperSha256 `
  -ArtifactManifest $freshManifest -ArtifactManifestSha256 $freshManifestSha256 `
  -UsbInterfaceIndex $usbInterfaceIndex -UsbDeviceInstanceSha256 $usbDeviceHash `
  -PcUsbAddress $pcUsbAddress -PhoneUsbAddress $phoneUsbAddress -Port $port `
  -PythonPath $pythonPath -WorkTimeoutSeconds 90 -InspectSeconds 10
```

The four helper basenames are `vw-editor-probe-runner.exe`,
`vw-editor-probe.exe`, `vw-editor-frame-runner.exe` and `vw-capture-helper.exe`.
Keep the actual mouse stationary outside the observed canvas during sampling.
The parent requires the native Brush witness and a stable white baseline before
phone input. It never adopts an existing/delegated Paint window or fabricates
an editor contract. Ordinary foreground acquisition has a five-second limit;
new-window readiness has a 15-second limit. Local preparation and post-sampling
each have a 90-second work budget, and helpers have 30-second limits. The phone
journey retains its existing work limits. Actual Pending input/Job/IO retirement
retains the destination and may outlive work limits; do not wrap this parent in
a destructive process timeout.

Success requires phone grant/ACK/render/retirement evidence, two identical opaque
white canvas samples followed by two identical changed canvas samples, exact
PNG/RGBA hash bindings, and no changed pixels outside the small stroke region.
The result remains visible for ten seconds, then only the task-owned Paint Job
is retired and private fixture/capture files are disposed. Inspect the fresh
`.local/paint-phone-visible-<run>.json`: require `status=passed`,
`editor_effect=true`, both Job/IO cleanup flags and `private_images_disposed=true`.
This establishes a software-generated phone stroke's observed Paint pixel effect;
physical S Pen, latency and normal Workbench UI acceptance remain separate.

Cloud validation: 76 distinct Python tests passed, including 15 new pixel/PNG
cases, plus Python compilation, the three setup-policy checks and the 57-path
artifact source closure check. Sol 6.1 implemented the parent and evaluator;
independent Astra review accepted ownership, retirement and evidence semantics
with no remaining source blockers.
PowerShell/C# compilation and the actual Windows/S23 run are pending locally;
this Linux environment has no cached PowerShell/.NET compiler. No GUI or device
execution is claimed by these software checks.

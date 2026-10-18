# Root producer draft preservation

This branch preserves work in progress separately from editing correction707e940.
The unchanged reviewed af934061 producer leaves compose as2ca1f33. Subsequent
root source is UNCOMPILED and UNRUN; it is not admitted into a reviewed candidate
or main. Only Rust source formatting and whitespace checks ran while the image
owner held the heavy slot.

Draft root work aggregates all four PublishedProject parts before fallible
construction and retains the32MiB charge last in actual ProjectState. Three
written/unrun cases cover failed adoption, cancelled close and abandoned startup
replies. The root-only Windows dependency edge is written; its lock composition
still needs to be resolved and verified without changing external pins.

The new Android draft invokes a fixed private JVM runner from the retained native
input worker. A bounded operation registry supplies cancellation probes; only a
single nested callback on that native worker can consume the real private typed
AndroidTileBuild. It holds withRootOutput while copying/validating tiles into an
independent native directory, and withholds the ApprovedBuild until Java returns
after successful actual output cleanup/charge retirement. No completed-directory
or runner registration API is exported. Its JNI types, Kotlin source, cancellation,
cleanup and memory behavior still require actual phase compilation, meaningful
regressions and Astra review. This draft does not activate a user import workflow.

Do not integrate image repair9b05bea before its separate review verdict. Preserve
its four files unchanged while repairing editing lifecycle findings on the
separate editing correction branch. The exact707e940 editing candidate must not
be promoted until both confirmed P2 findings and focused regressions pass recheck.
No build, binding regeneration, device, runtime pin, install, signing, provider,
identity or hardware gate change is claimed by this preserved draft.

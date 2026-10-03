# Provisional Rust stroke engine, algorithm 1

This is the software candidate for D6. Synthetic tests and CPU timings do not
establish S23/S Pen latency, owner preference, front-buffer performance, or final
engine selection. The owner-recording directory still has no pen traces.

`StrokeBuilder::begin(Brush)` validates a versioned brush. `append(&[Sample])`
validates an entire batch before changing state and returns `GeometryRange {
start, end }`, an appended range of polygons. Earlier polygons never change.
`finish()` closes the stroke, retains its readable geometry and returns a copy;
`into_geometry(self)` transfers ownership without that copy. Neither operation
changes vertices. `cancel()` clears provisional geometry and closes the builder.
A cloned builder can append temporary predicted samples; discard that clone and
keep prediction out of the authoritative saved model. Cloning copies contours,
so callers must bound prediction size.

All contours must be placed in **one NONZERO filled path and painted once**.
Separate painting of overlapping contours would over-darken alpha/highlighter
ink. The output is a union of convex contours: round nib disks and their common
external tangent quads, or flat-ended highlighter quads with round interior
joins. The highlighter uses butt caps; a single stationary sample has no segment
and therefore no filled geometry. Pen/marker taps produce a disk. The default
highlighter and vector eraser use constant pressure curves; callers can provide
another validated curve. Highlighter multiply blending belongs to the renderer.

The pressure curve is a monotonic cubic Bezier with x endpoints 0 and 1 and
nondecreasing x/y controls. Its inverse uses 48 fixed bisections. Stabilization
zero is unfiltered; positive stabilization uses a causal adaptive low-pass for
velocity, position and pressure. Equal timestamps use a fixed 1/240-second
interval. No wall clock or append batching affects geometry. Optional tilt and
orientation are checked and mapped from model arrays, but circular algorithm-1
nibs do not deform with these axes. Pressure and optional axes are canonicalized
to protobuf f32 precision before all modeling arithmetic uses f64.

All transcendental functions and quantization rounding use the pinned `libm`.
Round contour tessellation targets a pre-quantization sagitta of at most 1/8
D-pixel, with at least 12 vertices and exact axis extrema. Vertices are rounded
to signed integer 1/256 D-pixel units. An integer convex hull removes degenerate
or concave quantization artifacts. Distance and hit tests use these actual filled
polygons, with a bounding-box rejection for hit tests.

Limits are explicit: coordinates ±1,000,000,000 D-pixels; width (0,4096]; one
million accepted samples; four million output vertices. Oversized/invalid batches
return typed errors with the existing stroke unchanged. Geometry hashing uses a
versioned domain and little-endian integers, never platform serialization or
floating-point formatting.

`fixtures/synthetic.rs` is generated numeric data only. The CLI at
`tools/stroke-spike` emits observed hashes with `goldens` and bounded append CPU
timings with `benchmark [10..500]`. Populate `fixtures/goldens.json` from a reviewed
native `goldens` run before acceptance; the golden test deliberately rejects a
missing or incomplete expected-hash set. Run the identical committed corpus on
Windows and the authorized phone, and retain the separate device provenance.

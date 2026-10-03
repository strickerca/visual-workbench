# Cross-language fixture

`vw-ffi-golden` writes `ffi-golden.json` and `ffi-golden.properties` here from the
real Rust FFI API. The synthetic 64×64 source contains no captures or personal
data. Both target tests consume these same exact bytes, IDs, clocks, device ID,
state hash and PNG hash. A missing fixture fails the tests; it is not skipped.

Run the generator only through the parent's serialized build lane. Supply this
existing output directory and a fresh absolute temporary project path. Review
the generated text diff before retaining it as a golden; never regenerate to
hide a mismatch. The source PNG and result PNG are hex encoded text, not
verification screenshots. Source pixel arrays are not stored in log output.

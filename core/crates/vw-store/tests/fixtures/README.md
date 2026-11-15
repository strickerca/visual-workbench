# Generated video fixture

`short.mp4` contains 30 synthetic gray 64x64 frames, H.264 in MP4, lasting
1,000 ms. Generated on 2026-10-02 with the `generate_video` example using the
Windows Media Foundation sink writer. A separate source reader decoded all 30
frames and checked each decoded buffer length before generation was accepted.

- Bytes: 2,816
- SHA-256: `7e58e0b597d62afbebf6217c494a59311d269c9e24c7058e71a09f9a711d4c80`
- BLAKE3: `b491e1e9ef1c7a62611e895878799dbff1e8f61a389ab5df7d65ac979b3ec40c`
- Content and generator are project-owned; no external media or screen capture.

Regenerate to an **absent** output path with:
`cargo +1.99.0 run -p vw-store --example generate_video -- <new-path.mp4>`.
Container metadata/encoder versions may change the bytes; regeneration must be
revalidated and the hashes updated deliberately. Android storage tests use this
same checked-in fixture; they do not claim Android codec playback acceptance.

# Stored logs of an older build

`replay-host.sqlite` and `replay-mirror.sqlite` are the `project.sqlite` files
of two projects as the build of commit `a1f160eb` (2026-11-13) stored them:
the last commit before the engine was rewritten to decide transactions in
place. They hold the two-device script in `tests/replay_rule/script.rs`, the
host's project and the other device's, with that build's checkpoints (one at
every request, cancellation and catch-up) and no log seal. A project opens by
deciding its stored transactions again, so `tests/replay_rule.rs` opens these
with the current build and compares the result with what the script gives
today.

- `replay-host.sqlite`: 364,544 bytes, SHA-256
  `bc6896dd9a48b018f35f2a7307f09496c62e857e7ac853ffc3a3c7c228ae2bc3`
- `replay-mirror.sqlite`: 438,272 bytes, SHA-256
  `98ac4df022558311380b6c698d712bba287b3ded1541d9faba153334f36ceb86`
- Every identifier, device and time in them is synthetic and comes from the
  script; nothing was recorded from a person's project.

They are not to be regenerated when a test fails: a failure means the current
engine decides a stored transaction differently from the build that stored
it. To add a fixture for another old build, check that build out into a
separate directory, copy `tests/replay_rule/script.rs` into its
`core/crates/vw-store/tests/replay_rule/`, add a test beside it that calls
`script::run(<an absent directory>, |store, now| store.snapshot_now(now).unwrap())`
(builds from 2026-11-15 on call it `checkpoint_now`), run that test, and take
`host.vwb/project.sqlite` and `mirror.vwb/project.sqlite` from the directory.

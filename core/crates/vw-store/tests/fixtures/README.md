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

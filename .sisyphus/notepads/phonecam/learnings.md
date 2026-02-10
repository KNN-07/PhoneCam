## 2026-02-10

- For this protocol crate, `bincode` v2 + `serde` works cleanly for per-variant payload encoding/decoding, while the 1-byte wire type discriminant is handled manually by framing.
- `bytes::Bytes` with the `serde` feature preserves H.264 NAL byte payloads exactly and avoids unnecessary copies at the API boundary.
- A robust framing parser should verify both the 4-byte big-endian length prefix and full payload consumption after decode to catch trailing-byte corruption early.

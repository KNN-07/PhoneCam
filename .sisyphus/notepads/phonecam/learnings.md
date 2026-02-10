## 2026-02-10

- For this protocol crate, `bincode` v2 + `serde` works cleanly for per-variant payload encoding/decoding, while the 1-byte wire type discriminant is handled manually by framing.
- `bytes::Bytes` with the `serde` feature preserves H.264 NAL byte payloads exactly and avoids unnecessary copies at the API boundary.
- A robust framing parser should verify both the 4-byte big-endian length prefix and full payload consumption after decode to catch trailing-byte corruption early.

- For TCP transport, splitting `TcpStream` into owned read/write halves plus a bounded `mpsc` outbound queue cleanly enforces backpressure (`send().await` blocks when full).
- A simple app-level keepalive can be implemented with periodic `StatusUpdate("__phonecam_ping__")` and `timeout(read_message, 5s)` on the receive loop; no separate ping message type is required.
- Auto-sending a default `Handshake` immediately after connect/accept makes state transitions deterministic (`Handshaking -> Streaming`) and stabilizes lifecycle tests.

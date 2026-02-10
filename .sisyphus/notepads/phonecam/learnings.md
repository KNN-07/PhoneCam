## 2026-02-10

- For this protocol crate, `bincode` v2 + `serde` works cleanly for per-variant payload encoding/decoding, while the 1-byte wire type discriminant is handled manually by framing.
- `bytes::Bytes` with the `serde` feature preserves H.264 NAL byte payloads exactly and avoids unnecessary copies at the API boundary.
- A robust framing parser should verify both the 4-byte big-endian length prefix and full payload consumption after decode to catch trailing-byte corruption early.

- For TCP transport, splitting `TcpStream` into owned read/write halves plus a bounded `mpsc` outbound queue cleanly enforces backpressure (`send().await` blocks when full).
- A simple app-level keepalive can be implemented with periodic `StatusUpdate("__phonecam_ping__")` and `timeout(read_message, 5s)` on the receive loop; no separate ping message type is required.
- Auto-sending a default `Handshake` immediately after connect/accept makes state transitions deterministic (`Handshaking -> Streaming`) and stabilizes lifecycle tests.

- In `mdns-sd`, creating `ServiceInfo` with `ip=""` plus `.enable_addr_auto()` allows the daemon to advertise host interface addresses automatically (useful for IPv4/IPv6 and multi-interface support).
- `ServiceEvent::ServiceResolved` returns `ScopedIp` values; converting each with `to_ip_addr()` is the cleanest way to surface standard `IpAddr` values in crate APIs.
- For deterministic mDNS tests, injecting a custom daemon port (`ServiceDaemon::new_with_port`) for both publisher and browser avoids collisions with system mDNS services.

## 2026-02-10 — TODO 7 (iOS + Rust FFI)

- UniFFI CLI installation is via `cargo install uniffi --features cli` (binary `uniffi-bindgen`), not `cargo install uniffi_bindgen`.
- For Linux-hosted iOS cross-compilation, setting crate type to `staticlib`/`rlib` avoids `cdylib` linker failures that require Apple SDK tooling.
- A pragmatic iOS bootstrap flow is: Rust static libs for `aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios` → UniFFI Swift generation → optional `lipo` merge for simulator → `xcodebuild -create-xcframework` when Xcode tools exist.
- Generated UniFFI Swift bindings can coexist with a raw C frame-path ABI by importing generated `*FFI.h` in a project bridging header.

- For Tauri v2 desktop apps, `src-tauri` should be the Rust crate root. `tauri.conf.json` schema v2 places `identifier` at the top level, removing it from `bundle` object.
- Validating Tauri frontend UI with Playwright + Vite dev server (`npm run dev`) works even if the Rust backend fails to build due to missing system dependencies. This decouples UI verification from backend compilation in CI.

## 2026-02-10 (Wave 2)

- UniFFI Swift binding generation via `uniffi-bindgen generate --language swift <udl-file> --out-dir <output>` produces `.swift`, `.h`, and `.modulemap` files that can be directly imported into Xcode projects.
- For iOS cross-compilation, creating a shell script (`build-xcframework.sh`) that automates `cargo build` for all targets (`aarch64-apple-ios`, `aarch64-apple-ios-sim`, `x86_64-apple-ios`) plus `lipo` merging and `xcodebuild -create-xcframework` significantly streamlines mobile library builds.
- Tauri v2 desktop apps integrate cleanly with existing Rust crates via workspace dependencies — commands use `#[tauri::command]` macro and tokio for async operations.
- Sharing a single `phonecam-mobile-core` crate between Android and iOS via `crate-type = ["staticlib", "rlib"]` and platform-specific binding generation (UniFFI for Kotlin/Swift, raw `extern "C"` for frame data) successfully decouples cross-platform mobile logic from native UI code.

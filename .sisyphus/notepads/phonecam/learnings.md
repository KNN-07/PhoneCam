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

## 2026-02-11 (Wave 2 - Android)

- cargo-ndk integration in Gradle: Create `buildRustAndroid` task that invokes `cargo ndk -t arm64-v8a -t armeabi-v7a -t x86_64 -o <output-dir> build --release` to cross-compile for all Android architectures and place shared libraries in correct JNI directory structure.
- UniFFI Kotlin binding generation: `uniffi-bindgen generate <udl-file> --language kotlin --out-dir <dir>` produces `.kt` files that can be added to Kotlin source sets via `kotlin.sourceSets.getByName("main").kotlin.srcDir()`.
- For Android unit tests that call Rust FFI, build host cdylib target and set JNA_LIBRARY_PATH to `target/debug` so JNA can load the native library in JVM context.
- Android MainActivity uses JNA `Native.load()` to load Rust shared library and call raw extern "C" functions, with manual memory management for C string returns (call `phonecam_string_free()`).

## 2026-02-11 (Wave 3 - Linux Driver)

- v4l2loopback module detection via `/sys/module/v4l2loopback` path check is reliable and doesn't require root.
- The `v4l` crate provides safe Rust bindings for V4L2 ioctls via `context::enum_devices()` for device enumeration.
- Structured error types with installation instructions per distro (Ubuntu/Debian, Fedora, Arch) improve UX when module not loaded.
- Minimal stub implementation strategy: module detection + error handling first, actual frame writing deferred until H.264 decode pipeline exists.

## 2026-02-11 (Wave 3 - Video Pipeline)

- ffmpeg-next crate provides safe Rust bindings for FFmpeg decoder initialization and frame decoding
- H.264 decoder stub pattern: Define types (Nv12Frame, DecodeOutput) with TDD tests containing sample H.264 Annex-B NAL units, then implement in "green" phase
- CameraX on Android: Use ImageAnalysis with KEEP_ONLY_LATEST backpressure for low-latency frame capture, avoiding buffering delays
- MediaCodec H.264 encoder: Configure with KEY_BITRATE_MODE_CBR, KEY_I_FRAME_INTERVAL=1, KEY_PROFILE=AVCProfileBaseline for consistent streaming
- AVFoundation iOS: Use AVCaptureVideoDataOutput with dedicated serial queue for frame capture, AVCaptureSession.Preset for resolution
- VideoToolbox: Create VTCompressionSession with kVTCompressionPropertyKey_RealTime=true and kVTCompressionPropertyKey_AllowFrameReordering=false for low latency
- Rust FFI pattern: Swift/Kotlin pass raw frame buffers to phonecam_send_video_frame C FFI, Rust wraps in protocol::VideoFrame and sends via transport

## 2026-02-11 (Wave 4 - E2E Integration)

- Tauri pipeline manager pattern: Spawn long-running async worker with `tokio::spawn`, manage via Arc<TokioMutex<Runtime>> holding shutdown channel + JoinHandle
- Use `watch::channel<PipelineStatus>` for observable state that UI can subscribe to, with state transitions driven by pipeline lifecycle events
- Pipeline worker lifecycle: mDNS advertise → TCP bind → accept (with shutdown select) → stream loop → disconnect → cleanup
- Main streaming select loop multiplexes: shutdown signal, connection state monitoring, message receive, enabling clean cancellation at any point
- Lazy converter initialization: Create Nv12ToYuyvConverter only when first frame arrives and dimensions are known, recreate on resolution change
- v4l2 device format setting: Call `device.set_format(width, height, PixelFormat::YUYV)` when resolution changes, before writing frames
- Environment variable `PHONECAM_V4L2_DEVICE` allows manual override of default v4l2 device selection for testing/debugging
- Error handling strategy: Decode/convert failures log warnings but continue (allow pipeline to recover), device failures return Err and exit stream loop

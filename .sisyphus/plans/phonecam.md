# PhoneCam — Phone as Webcam Application

## TL;DR

> **Quick Summary**: Build a cross-platform open-source application that turns a phone into a virtual webcam for desktop. Consists of a Rust core library (protocol, transport, discovery), native mobile apps (Swift/Kotlin with Rust FFI), and a Tauri desktop app with platform-specific virtual webcam drivers.
>
> **Deliverables**:
> - `phonecam-protocol` — Rust crate: wire format, message types, H.264 NAL framing
> - `phonecam-transport` — Rust crate: TCP transport, connection management
> - `phonecam-discovery` — Rust crate: mDNS service publish/discover
> - `phonecam-desktop` — Tauri app: GUI + H.264 decode + driver management
> - `phonecam-driver-linux` — Rust crate: v4l2loopback integration
> - `phonecam-driver-macos` — Swift Xcode project: CMIO Camera Extension
> - `phonecam-driver-windows` — C++ project: Frame Server Custom Media Source / DirectShow filter
> - `phonecam-android` — Kotlin app: CameraX + MediaCodec + Rust FFI
> - `phonecam-ios` — Swift app: AVFoundation + VideoToolbox + Rust FFI
>
> **Estimated Effort**: XL
> **Parallel Execution**: YES — 5 waves
> **Critical Path**: Protocol → Transport → Desktop Shell + H.264 Decode → Linux Driver → Android App → E2E Integration

---

## Context

### Original Request
Build a phone-as-webcam application with two parts: Desktop Client (cross-platform: Windows, macOS, Linux — installs virtual webcam, GUI for resolution/FPS control) and Mobile Client (cross-platform: iOS, Android — streams camera to desktop). Multiple connection methods (wired USB, wireless WiFi).

### Interview Summary
**Key Discussions**:
- **Language**: Rust for core logic. User's primary language and preferred ecosystem.
- **Mobile strategy**: Native UI (Swift/Kotlin) + shared Rust core via FFI. Best camera control, best latency. Encoding happens in native code (VideoToolbox/MediaCodec), NOT in Rust.
- **Desktop GUI**: Tauri (Rust backend + web frontend). Lightweight, cross-platform, natural Rust companion.
- **Streaming protocol**: Custom TCP/UDP with H.264 video + length-prefixed NAL units. Inspired by scrcpy (proven 35-70ms latency). NOT WebRTC — both devices are same network or USB, no NAT traversal needed.
- **Discovery**: mDNS auto-discovery + QR code fallback for networks where mDNS doesn't work.
- **USB wired**: ADB port forwarding for Android (proven by scrcpy). iOS WiFi-only in v1 (usbmuxd/Peertalk is poorly documented, no Rust crate).
- **Audio**: User requested audio support. Virtual audio drivers are each project-sized efforts (kernel WDM on Windows, AudioDriverKit on macOS). v1 defers custom virtual audio drivers — recommend existing solutions (snd-aloop, BlackHole, VB-Cable). Protocol reserves audio message types.
- **Camera controls**: Front/back camera switching in v1. Advanced controls (zoom, exposure, white balance) deferred to v2.
- **Scope**: Core camera streaming + front/back switch. NO AI features, recording, screen sharing, auto-reconnection.
- **Project structure**: Monorepo with Cargo workspace.
- **Platform priority**: User wants all platforms from day 1. Plan covers all but phases execution for testability.
- **License**: Apache 2.0.
- **Test strategy**: TDD with `cargo test` for Rust crates.

**Research Findings**:
- scrcpy (Apache-2.0): Reference architecture for H.264 + ADB streaming, 35-70ms latency
- v4l2loopback: Linux standard virtual webcam, `exclusive_caps=1` required for Chrome/WebRTC
- macOS CMIO Camera Extensions (macOS 12.3+): Must be Swift/Obj-C, runs as separate sandboxed process, needs IPC for frame delivery
- Windows: Frame Server Custom Media Source (Win11) or DirectShow filter (Win10). Requires C++ COM DLL.
- Rust ecosystem: `v4l` crate (MIT), `ffmpeg-next`, `nokhwa` (MIT/Apache), `mdns-sd` for mDNS
- Mobile H.264 encoding MUST use native platform APIs (VideoToolbox on iOS, MediaCodec on Android) — cannot be called from Rust
- Firezone project: Best reference for Rust monorepo + native mobile via UniFFI
- UniFFI: Use for config/control APIs. Use raw `extern "C"` FFI for high-frequency frame data path.

### Metis Review
**Identified Gaps** (addressed):
- **H.264 encoding cannot happen in Rust on mobile**: Corrected — native code encodes, Rust core receives pre-encoded bytes
- **macOS CMIO Extension must be Swift**: Corrected — separate Xcode sub-project, not a Rust crate
- **Windows driver must be C++**: Corrected — separate C++ project with Rust FFI bridge
- **Virtual audio drivers are project-sized**: Deferred to v2, recommend existing solutions
- **iOS USB (usbmuxd) is too complex for v1**: iOS WiFi-only in v1
- **UniFFI async has issues on Android**: Use raw FFI for frame data, UniFFI for config only
- **Phone orientation changes break drivers**: Lock orientation in v1
- **v4l2loopback requires user action**: Add detection + guidance UX
- **All platforms simultaneously needs phasing**: Execution waves ensure testability while covering all platforms

---

## Work Objectives

### Core Objective
Deliver a functional open-source phone-as-webcam system where a phone streams its camera to a desktop computer, which presents it as a virtual webcam usable by any video conferencing app (Zoom, Teams, Meet, OBS).

### Concrete Deliverables
- Rust monorepo with Cargo workspace containing protocol, transport, discovery crates
- Tauri desktop app for Windows, macOS, Linux with virtual webcam driver integration
- Android app (Kotlin) that captures camera and streams H.264 to desktop
- iOS app (Swift) that captures camera and streams H.264 to desktop (WiFi only in v1)
- Platform-specific virtual webcam drivers: v4l2loopback (Linux), CMIO Camera Extension (macOS), DirectShow/Frame Server (Windows)
- mDNS auto-discovery + QR code fallback connection
- Android USB support via ADB port forwarding
- Front/back camera switching

### Definition of Done
- [ ] Android phone streams camera → Linux desktop → virtual webcam → `ffprobe` reads frames from `/dev/videoN`
- [ ] Android phone streams camera → Windows desktop → virtual webcam → OBS/ffprobe reads frames
- [ ] Android phone streams camera → macOS desktop → virtual webcam → FaceTime/ffprobe reads frames
- [ ] iOS phone streams camera → all desktops (WiFi only)
- [ ] mDNS discovery finds desktop within 3 seconds on same subnet
- [ ] QR code fallback connects when mDNS unavailable
- [ ] Android USB (ADB) streaming works with ≤70ms end-to-end latency
- [ ] Front/back camera switch works mid-stream without disconnect
- [ ] All Rust crate tests pass: `cargo test --workspace`
- [ ] Resolution configurable: 480p, 720p, 1080p at 15/30/60 FPS

### Must Have
- Custom TCP protocol with H.264 NAL unit framing + timestamp sync
- mDNS service discovery (zero-config WiFi connection)
- Virtual webcam driver per desktop platform
- Native hardware H.264 encoding on mobile (no software encoding)
- Front/back camera switching
- Configurable resolution and FPS
- v4l2loopback detection + user guidance on Linux
- Apache 2.0 license throughout

### Must NOT Have (Guardrails)
- ❌ No WebRTC, STUN/TURN/ICE, DTLS — protocol is simple TCP/UDP for local network
- ❌ No AI features (background blur, virtual backgrounds, filters) — v2
- ❌ No custom virtual audio drivers — recommend existing solutions (snd-aloop, BlackHole, VB-Cable)
- ❌ No iOS USB support — WiFi only in v1 (usbmuxd/Peertalk deferred)
- ❌ No auto-reconnection — manual reconnect on disconnect in v1
- ❌ No software H.264 encoding on mobile (x264/openh264) — battery/thermal death
- ❌ No recording, screen sharing, or cloud relay
- ❌ No advanced camera controls (zoom, exposure, white balance) — v2, front/back switch only
- ❌ No Rust code calling VideoToolbox/MediaCodec — encoding stays in native mobile code
- ❌ No UniFFI for high-frequency frame data — use raw `extern "C"` FFI for frame buffers
- ❌ No multi-phone support (multiple cameras from different phones)
- ❌ No installer/distribution packaging in v1 — developer builds only

---

## Verification Strategy

> **UNIVERSAL RULE: ZERO HUMAN INTERVENTION**
>
> ALL tasks MUST be verifiable WITHOUT human action, with explicit exceptions for platform-specific driver registration that requires real hardware.
>
> **Platform-Specific Exceptions (marked per-task):**
> - macOS CMIO Extension registration — requires macOS + code signing
> - Windows COM driver registration — requires Windows 11
> - Real phone camera capture quality — requires physical device
> - Zoom/Teams/Meet compatibility — requires manual testing
>
> These are explicitly marked as "manual verification required" in affected tasks.

### Test Decision
- **Infrastructure exists**: NO (greenfield)
- **Automated tests**: TDD (test-first)
- **Framework**: `cargo test` (built-in Rust testing) for all Rust crates
- **Android**: JUnit + instrumented tests for Kotlin code
- **iOS**: XCTest for Swift code

### TDD Structure (Rust Crates)
Each Rust task follows RED-GREEN-REFACTOR:
1. **RED**: Write failing test first in `tests/` or `#[cfg(test)]` module
2. **GREEN**: Implement minimum code to pass
3. **REFACTOR**: Clean up while keeping green
4. **Verify**: `cargo test -p <crate-name>` → all PASS

### Agent-Executed QA Scenarios
Every task includes QA scenarios using:
- **Rust crates**: `cargo test` assertions
- **Linux driver**: `v4l2-ctl`, `ffprobe` against virtual device
- **Tauri desktop**: Playwright for web frontend UI
- **Android**: ADB commands, `adb forward`, emulator testing where possible
- **macOS/Windows/iOS**: Marked as manual verification where agent cannot test

---

## Execution Strategy

### Parallel Execution Waves

```
Wave 1 (Foundation — Start Immediately):
├── TODO 1: Monorepo skeleton + CI + TDD infra
├── TODO 2: phonecam-protocol crate
├── TODO 3: phonecam-transport crate
└── TODO 4: phonecam-discovery crate

Wave 2 (Platform Skeleton — After Wave 1):
├── TODO 5: Tauri desktop app skeleton
├── TODO 6: Android project + Rust FFI setup
└── TODO 7: iOS project + Rust FFI setup

Wave 3 (Drivers + Camera — After Wave 2):
├── TODO 8: Linux v4l2loopback driver integration
├── TODO 9: Desktop H.264 decode pipeline (ffmpeg-next)
├── TODO 10: macOS CMIO Camera Extension (Swift)
├── TODO 11: Windows virtual camera driver (C++)
├── TODO 12: Android camera capture + H.264 encoding
└── TODO 13: iOS camera capture + H.264 encoding

Wave 4 (Integration — After Wave 3):
├── TODO 14: Linux + Android WiFi E2E integration
├── TODO 15: macOS desktop integration (Tauri + CMIO IPC)
├── TODO 16: Windows desktop integration (Tauri + COM driver)
├── TODO 17: iOS WiFi integration
├── TODO 18: Android USB (ADB) integration
└── TODO 19: mDNS discovery + QR code fallback integration

Wave 5 (Polish — After Wave 4):
├── TODO 20: Tauri settings frontend (resolution/FPS/camera controls UI)
├── TODO 21: Camera front/back switching protocol + implementation
├── TODO 22: Error handling + connection state management
└── TODO 23: Cross-platform CI/CD pipeline

Critical Path: TODO 1 → TODO 2 → TODO 3 → TODO 5 → TODO 9 → TODO 8 → TODO 14
Parallel Speedup: ~60% faster than sequential (Waves 3 and 4 have 5-6 parallel tasks)
```

### Dependency Matrix

| Task | Depends On | Blocks | Can Parallelize With |
|------|-----------|--------|---------------------|
| 1 | None | 2,3,4 | None (first) |
| 2 | 1 | 3,5,6,7 | None (defines wire format) |
| 3 | 2 | 5,6,7 | None (needs protocol) |
| 4 | 1 | 19 | 2,3 |
| 5 | 3 | 8,9,14-16,20 | 6,7 |
| 6 | 3 | 12,14,18 | 5,7 |
| 7 | 3 | 13,17 | 5,6 |
| 8 | 5 | 14 | 9,10,11,12,13 |
| 9 | 5 | 8,14,15,16 | 8,10,11,12,13 |
| 10 | 2 | 15 | 8,9,11,12,13 |
| 11 | 2 | 16 | 8,9,10,12,13 |
| 12 | 6 | 14,18 | 8,9,10,11,13 |
| 13 | 7 | 17 | 8,9,10,11,12 |
| 14 | 8,9,12 | 20,21,22 | 15,16,17,18,19 |
| 15 | 5,9,10 | 20,22 | 14,16,17,18,19 |
| 16 | 5,9,11 | 20,22 | 14,15,17,18,19 |
| 17 | 5,9,13 | 22 | 14,15,16,18,19 |
| 18 | 6,12 | 22 | 14,15,16,17,19 |
| 19 | 4,5,6,7 | 22 | 14,15,16,17,18 |
| 20 | 14 | 22 | 21 |
| 21 | 14 | 22 | 20 |
| 22 | 14-19 | 23 | 20,21 |
| 23 | 22 | None | None (final) |

### Agent Dispatch Summary

| Wave | Tasks | Recommended Agents |
|------|-------|--------------------|
| 1 | 1-4 | Sequential: `category="unspecified-high"` |
| 2 | 5-7 | Parallel: `category="unspecified-high"` |
| 3 | 8-13 | Parallel (6 tasks): `category="ultrabrain"` for drivers, `category="unspecified-high"` for mobile |
| 4 | 14-19 | Parallel (6 tasks): `category="unspecified-high"` |
| 5 | 20-23 | Mixed: `category="visual-engineering"` for UI, `category="unspecified-high"` for others |

---

## TODOs

> Implementation + Test = ONE Task. Never separate.
> EVERY task has: Recommended Agent Profile + Parallelization info.

---

- [x] 1. Monorepo Skeleton + CI + TDD Infrastructure

  **What to do**:
  - Create Cargo workspace with member crates: `phonecam-protocol`, `phonecam-transport`, `phonecam-discovery`, `phonecam-desktop`, `phonecam-driver-linux`
  - Create directory structure: `/rust/` (Cargo workspace), `/android/` (Gradle project placeholder), `/ios/` (Xcode project placeholder), `/apple/` (macOS driver placeholder), `/windows/` (C++ driver placeholder)
  - Set up `Cargo.toml` workspace with shared dependencies (serde, tokio, thiserror)
  - Create `.github/workflows/ci.yml` with `cargo test --workspace`, `cargo clippy`, `cargo fmt --check`
  - Add `LICENSE` (Apache 2.0), `.gitignore`, basic `README.md`
  - Create first test in `phonecam-protocol` to verify TDD pipeline: `#[test] fn placeholder() { assert!(true); }`

  **Must NOT do**:
  - Do NOT create any mobile/desktop project files yet (just placeholder directories)
  - Do NOT add complex CI (just Rust checks for now)

  **Recommended Agent Profile**:
  - **Category**: `quick`
  - **Skills**: [`git-master`]
    - `git-master`: Proper initial commit structure

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 — Sequential (must be first)
  - **Blocks**: 2, 3, 4
  - **Blocked By**: None

  **References**:
  - **Pattern**: Firezone monorepo structure — `/rust` for crates, `/apple` and `/android` for native: https://github.com/firezone/firezone
  - **Pattern**: Spacedrive Cargo workspace — large Tauri monorepo reference: https://github.com/spacedriveapp/spacedrive
  - **Docs**: Cargo workspace docs: https://doc.rust-lang.org/book/ch14-03-cargo-workspaces.html

  **Acceptance Criteria**:
  - [ ] `cargo test --workspace` runs and passes (1+ placeholder test)
  - [ ] `cargo clippy --workspace` passes with no warnings
  - [ ] `cargo fmt --workspace --check` passes
  - [ ] Directory structure exists: `rust/`, `android/`, `ios/`, `apple/`, `windows/`
  - [ ] `LICENSE` file contains Apache 2.0 text
  - [ ] `.github/workflows/ci.yml` exists with correct steps

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Workspace builds and tests pass
    Tool: Bash
    Preconditions: Repository initialized
    Steps:
      1. cargo test --workspace 2>&1
      2. Assert: exit code 0
      3. Assert: output contains "test result: ok"
      4. cargo clippy --workspace 2>&1
      5. Assert: exit code 0
      6. ls -la rust/ android/ ios/ apple/ windows/
      7. Assert: all directories exist
    Expected Result: Clean workspace with passing tests
    Evidence: Terminal output captured
  ```

  **Commit**: YES
  - Message: `chore: initialize monorepo with Cargo workspace and CI`
  - Files: `Cargo.toml`, `rust/*/Cargo.toml`, `.github/workflows/ci.yml`, `LICENSE`, `.gitignore`, `README.md`
  - Pre-commit: `cargo test --workspace`

---

- [x] 2. `phonecam-protocol` Crate — Wire Format + Message Types

  **What to do**:
  - **RED**: Write tests first for all message types and serialization round-trips
  - Define protocol message enum: `Handshake`, `VideoFrame`, `AudioFrame` (reserved), `CameraControl`, `StatusUpdate`, `Disconnect`
  - Define `Handshake` message: version, device name, supported resolutions, supported FPS values
  - Define `VideoFrame` message: H.264 NAL unit bytes, presentation timestamp (u64 microseconds), frame dimensions, is_keyframe flag
  - Define `AudioFrame` message (reserved, not implemented): codec type, sample rate, channels, audio bytes
  - Define `CameraControl` message: `SwitchCamera { front: bool }`, (other controls reserved for v2)
  - Implement length-prefixed binary framing: `[4-byte message length][1-byte message type][payload]`
  - Use `serde` + custom binary serialization (NOT JSON — binary for performance)
  - **GREEN**: Implement all types and serialization to pass tests
  - **REFACTOR**: Extract framing logic, ensure zero-copy where possible

  **Must NOT do**:
  - Do NOT implement audio frame handling (just define the message type, mark as reserved)
  - Do NOT use JSON or text-based serialization — binary only
  - Do NOT add compression — H.264 is already compressed
  - Do NOT add encryption in v1

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Pure Rust systems programming, no special skills needed

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 — Sequential after TODO 1
  - **Blocks**: 3, 5, 6, 7, 10, 11
  - **Blocked By**: 1

  **References**:
  - **Pattern**: scrcpy protocol — raw H.264 stream with 64-bit PTS prefix per packet: https://github.com/Genymobile/scrcpy/blob/master/app/src/decoder.c
  - **Docs**: H.264 NAL unit structure — Annex B start codes vs length-prefixed: https://yumichan.net/video-processing/video-codec/introduction-to-h264-nal-unit/
  - **Crate**: `bytes` crate for zero-copy buffer management: https://docs.rs/bytes
  - **Crate**: `serde` for derive macros on message types

  **Acceptance Criteria**:
  - [ ] TDD: Tests written FIRST, then implementation
  - [ ] `cargo test -p phonecam-protocol` → PASS (all message type round-trip tests)
  - [ ] All message types serialize/deserialize correctly (round-trip property test)
  - [ ] H.264 NAL unit framing preserves byte-exact content
  - [ ] Length-prefixed framing correctly handles messages up to 1MB
  - [ ] `AudioFrame` type exists but is marked `#[deprecated(note = "Reserved for v2")]`

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Protocol message round-trip serialization
    Tool: Bash (cargo test)
    Preconditions: phonecam-protocol crate exists
    Steps:
      1. cargo test -p phonecam-protocol -- --test-threads=1 2>&1
      2. Assert: exit code 0
      3. Assert: output contains "test result: ok"
      4. Assert: output shows tests for: handshake_roundtrip, video_frame_roundtrip, camera_control_roundtrip, framing_large_payload
    Expected Result: All protocol tests pass
    Evidence: Test output captured

  Scenario: Binary framing correctness
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p phonecam-protocol -- framing 2>&1
      2. Assert: test verifies [4-byte length][1-byte type][payload] format
      3. Assert: payloads from 0 bytes to 1MB handled correctly
    Expected Result: Framing handles all sizes
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(protocol): implement wire format with H.264 NAL framing and message types`
  - Files: `rust/phonecam-protocol/src/**`
  - Pre-commit: `cargo test -p phonecam-protocol`

---

- [x] 3. `phonecam-transport` Crate — TCP Transport + Connection Management

  **What to do**:
  - **RED**: Write tests for TCP connection lifecycle, message sending/receiving, concurrent streams
  - Implement `PhoneCamServer` (desktop side): listens on configurable port, accepts one client connection
  - Implement `PhoneCamClient` (mobile side): connects to server IP:port
  - Use `tokio` for async networking
  - Implement message framing over TCP using `phonecam-protocol` framing format
  - Implement connection state machine: `Disconnected → Connecting → Handshaking → Streaming → Disconnected`
  - Implement keepalive pings (every 1 second, timeout after 5 seconds → disconnect)
  - Expose async channel API: `sender.send(VideoFrame)` / `receiver.recv() -> Message`
  - **GREEN**: Implement transport to pass all tests
  - **REFACTOR**: Optimize for throughput, ensure backpressure handling

  **Must NOT do**:
  - Do NOT implement UDP transport in v1 — TCP only (reliable, simpler)
  - Do NOT implement auto-reconnection — just clean disconnect
  - Do NOT implement multiple simultaneous connections (one phone at a time)
  - Do NOT implement encryption/TLS in v1

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Async Rust networking, systems programming

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 1 — Sequential after TODO 2
  - **Blocks**: 5, 6, 7
  - **Blocked By**: 2

  **References**:
  - **Pattern**: scrcpy connection handling — TCP socket with non-blocking connect + timeout: https://github.com/dev47apps/droidcam-linux-client/blob/master/src/connection.c
  - **Crate**: `tokio` for async runtime: https://docs.rs/tokio
  - **Crate**: `tokio::net::TcpListener` and `TcpStream` for server/client
  - **Pattern**: Channel-based API similar to `tokio::sync::mpsc`

  **Acceptance Criteria**:
  - [ ] TDD: Tests written FIRST
  - [ ] `cargo test -p phonecam-transport` → PASS
  - [ ] Server listens, client connects, handshake completes
  - [ ] Video frame messages sent and received correctly over loopback
  - [ ] Sustained throughput ≥ 5 Mbps over loopback (1080p30 equivalent)
  - [ ] Transport-only latency < 5ms on loopback
  - [ ] Keepalive timeout correctly transitions to Disconnected state
  - [ ] Connection state machine transitions are correct and tested

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: TCP transport throughput test
    Tool: Bash (cargo test)
    Preconditions: phonecam-transport crate exists with phonecam-protocol dependency
    Steps:
      1. cargo test -p phonecam-transport -- --test-threads=1 2>&1
      2. Assert: exit code 0
      3. Assert: test "throughput_1080p30" passes (sends 30 frames/sec of ~150KB each)
      4. Assert: test "connection_lifecycle" passes (connect → handshake → stream → disconnect)
      5. Assert: test "keepalive_timeout" passes (no ping → disconnect after 5s)
    Expected Result: All transport tests pass
    Evidence: Test output captured

  Scenario: Latency measurement
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p phonecam-transport -- latency 2>&1
      2. Assert: measured round-trip latency < 10ms on loopback
    Expected Result: Low latency on loopback
    Evidence: Test output with timing data
  ```

  **Commit**: YES
  - Message: `feat(transport): implement TCP transport with async channels and connection state machine`
  - Files: `rust/phonecam-transport/src/**`
  - Pre-commit: `cargo test -p phonecam-transport`

---

- [x] 4. `phonecam-discovery` Crate — mDNS Service Discovery

  **What to do**:
  - **RED**: Write tests for service registration and discovery
  - Implement service publisher (desktop side): register `_phonecam._tcp.local.` with device name, port, version
  - Implement service browser (mobile side): discover all `_phonecam._tcp` services on local network
  - Use `mdns-sd` crate (MIT, pure Rust, cross-platform)
  - Return discovered services as: `{ name: String, ip: IpAddr, port: u16, version: String }`
  - Implement QR code data format: `phonecam://IP:PORT?name=DEVICE_NAME` (simple URI)
  - **GREEN**: Implement discovery
  - **REFACTOR**: Handle IPv4/IPv6, multiple network interfaces

  **Must NOT do**:
  - Do NOT implement QR code rendering/scanning (just the data format — UI handles rendering)
  - Do NOT implement service authentication

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 1 (with TODOs 2, 3 after TODO 1)
  - **Blocks**: 19
  - **Blocked By**: 1

  **References**:
  - **Crate**: `mdns-sd` — pure Rust mDNS/DNS-SD library: https://docs.rs/mdns-sd
  - **Spec**: DNS-SD RFC 6763: https://tools.ietf.org/html/rfc6763
  - **Pattern**: Bonjour/Avahi service type naming: `_servicename._tcp.local.`

  **Acceptance Criteria**:
  - [ ] TDD: Tests written FIRST
  - [ ] `cargo test -p phonecam-discovery` → PASS
  - [ ] Service published, discovered within 3 seconds on loopback
  - [ ] Service metadata includes IP, port, device name, version
  - [ ] QR code URI format `phonecam://IP:PORT?name=NAME` parses correctly
  - [ ] IPv4 and IPv6 addresses handled

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: mDNS publish and discover on loopback
    Tool: Bash (cargo test)
    Steps:
      1. cargo test -p phonecam-discovery -- mdns 2>&1
      2. Assert: exit code 0
      3. Assert: test "publish_and_discover" passes
      4. Assert: test "qr_code_uri_format" passes
    Expected Result: mDNS works on loopback
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(discovery): implement mDNS service discovery and QR code URI format`
  - Files: `rust/phonecam-discovery/src/**`
  - Pre-commit: `cargo test -p phonecam-discovery`

---

- [x] 5. Tauri Desktop App Skeleton

  **What to do**:
  - Initialize Tauri v2 project in `rust/phonecam-desktop/`
  - Set up web frontend with basic HTML/CSS/JS (or minimal React/Svelte — keep simple)
  - Create Rust backend commands: `connect(ip, port)`, `disconnect()`, `get_status()`, `get_discovered_devices()`
  - Integrate `phonecam-transport` as dependency for connection management
  - Integrate `phonecam-discovery` as dependency for device discovery
  - Create basic UI: device list (from mDNS), connect button, connection status indicator
  - Set up Tauri IPC between frontend and Rust backend
  - Ensure cross-platform build: `cargo tauri build` works on current OS

  **Must NOT do**:
  - Do NOT implement video display in the Tauri window (video goes to virtual driver, not GUI)
  - Do NOT implement settings UI yet (TODO 20)
  - Do NOT implement driver management yet (separate TODOs)

  **Recommended Agent Profile**:
  - **Category**: `visual-engineering`
  - **Skills**: [`frontend-ui-ux`]
    - `frontend-ui-ux`: Tauri web frontend design

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with TODOs 6, 7)
  - **Blocks**: 8, 9, 14, 15, 16, 20
  - **Blocked By**: 3

  **References**:
  - **Docs**: Tauri v2 getting started: https://v2.tauri.app/start/
  - **Pattern**: Spacedrive Tauri app structure: https://github.com/spacedriveapp/spacedrive
  - **Docs**: Tauri IPC commands: https://v2.tauri.app/develop/calling-rust/

  **Acceptance Criteria**:
  - [ ] `cargo tauri dev` launches the app window on current OS
  - [ ] Frontend shows "Discovered Devices" section (empty list initially)
  - [ ] Rust backend `get_status()` command returns `{ connected: false }`
  - [ ] `cargo test -p phonecam-desktop` → PASS

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Tauri app launches and shows UI
    Tool: Playwright (playwright skill)
    Preconditions: Tauri dev server running
    Steps:
      1. Launch Tauri app via cargo tauri dev
      2. Wait for window to appear (timeout: 15s)
      3. Assert: page title contains "PhoneCam"
      4. Assert: element with text "Discovered Devices" is visible
      5. Assert: element with text "Not Connected" or similar status indicator is visible
      6. Screenshot: .sisyphus/evidence/task-5-tauri-shell.png
    Expected Result: App launches with basic UI
    Evidence: .sisyphus/evidence/task-5-tauri-shell.png
  ```

  **Commit**: YES
  - Message: `feat(desktop): create Tauri app skeleton with device discovery UI`
  - Files: `rust/phonecam-desktop/**`
  - Pre-commit: `cargo test -p phonecam-desktop`

---

- [x] 6. Android Project + Rust FFI Setup

  **What to do**:
  - Create Android project in `android/` with Kotlin, Gradle, minSdk 26 (Android 8.0+)
  - Set up Rust cross-compilation for Android targets: `aarch64-linux-android`, `armv7-linux-androideabi`, `x86_64-linux-android`
  - Create `phonecam-mobile-core` Rust crate with UniFFI UDL for config/control APIs
  - Create raw `extern "C"` FFI interface for high-frequency frame data: `phonecam_send_video_frame(data: *const u8, len: usize, pts: u64, is_keyframe: bool)`
  - Set up `build.gradle` with Rust build integration (cargo-ndk or mozilla/rust-android-gradle)
  - Create basic Android app structure: `MainActivity`, single Activity with placeholder UI
  - Verify FFI works: call a simple Rust function from Kotlin, log result

  **Must NOT do**:
  - Do NOT implement camera capture yet (TODO 12)
  - Do NOT implement full UI yet (TODO 12)
  - Do NOT use UniFFI for frame data — raw FFI only for performance

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with TODOs 5, 7)
  - **Blocks**: 12, 14, 18
  - **Blocked By**: 3

  **References**:
  - **Pattern**: Firezone Android + Rust integration: https://github.com/firezone/firezone/tree/main/android
  - **Pattern**: matrix-rust-sdk UniFFI for Android: https://github.com/nicegram/nicegram-android/blob/main/nicegram-features/build.gradle
  - **Crate**: `uniffi` for config API bindings: https://mozilla.github.io/uniffi-rs/
  - **Tool**: `cargo-ndk` for Android cross-compilation: https://github.com/nicegram/nicegram-android
  - **Docs**: JNI and `#[no_mangle] extern "C"` for raw FFI: https://developer.android.com/ndk/guides/jni

  **Acceptance Criteria**:
  - [ ] Android project builds: `./gradlew assembleDebug` succeeds
  - [ ] Rust crate cross-compiles for `aarch64-linux-android`
  - [ ] UniFFI-generated Kotlin bindings exist and compile
  - [ ] Raw FFI function callable from Kotlin (verified by log output)
  - [ ] App installs and runs on Android emulator (shows placeholder screen)

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Android project builds with Rust FFI
    Tool: Bash
    Preconditions: Android SDK and NDK installed
    Steps:
      1. cd android && ./gradlew assembleDebug 2>&1
      2. Assert: exit code 0
      3. Assert: APK file exists at android/app/build/outputs/apk/debug/app-debug.apk
      4. ls -la android/app/build/outputs/apk/debug/
      5. Assert: APK size > 1MB (includes native libs)
    Expected Result: Android APK builds successfully with Rust native libraries
    Evidence: Build output captured
  ```

  **Commit**: YES
  - Message: `feat(android): set up Android project with Rust FFI via UniFFI and raw extern C`
  - Files: `android/**`, `rust/phonecam-mobile-core/**`
  - Pre-commit: `./gradlew assembleDebug`

---

- [x] 7. iOS Project + Rust FFI Setup

  **What to do**:
  - Create Xcode project in `ios/PhoneCam/` with Swift, minimum deployment target iOS 15.0
  - Set up Rust cross-compilation for iOS targets: `aarch64-apple-ios`, `aarch64-apple-ios-sim`
  - Reuse `phonecam-mobile-core` Rust crate (shared with Android)
  - Generate Swift bindings via UniFFI (`uniffi-bindgen generate --language swift`)
  - Create raw C FFI header for frame data functions (same interface as Android)
  - Build XCFramework containing Rust static library for all iOS architectures
  - Create basic app structure: single view placeholder UI
  - Verify FFI works: call Rust function from Swift, print result

  **Must NOT do**:
  - Do NOT implement camera capture yet (TODO 13)
  - Do NOT implement full UI yet
  - Do NOT implement USB support (WiFi only for iOS in v1)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 2 (with TODOs 5, 6)
  - **Blocks**: 13, 17
  - **Blocked By**: 3

  **References**:
  - **Pattern**: matrix-rust-sdk XCFramework build: https://github.com/nicegram/nicegram-ios/blob/main/nicegram-features/BUILD.md
  - **Pattern**: Firezone iOS + Rust integration: https://github.com/nicegram/nicegram-ios
  - **Crate**: `uniffi` Swift bindings: https://mozilla.github.io/uniffi-rs/swift/overview.html
  - **Tool**: `cargo-lipo` or manual `lipo` for fat binary creation

  **Acceptance Criteria**:
  - [ ] Xcode project builds for iOS Simulator target
  - [ ] Rust `phonecam-mobile-core` cross-compiles for `aarch64-apple-ios-sim`
  - [ ] UniFFI-generated Swift bindings compile in Xcode
  - [ ] Raw C FFI header imported and callable from Swift
  - [ ] App runs on iOS Simulator (shows placeholder screen)

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: iOS project builds with Rust FFI
    Tool: Bash
    Preconditions: Xcode and iOS SDK installed (MANUAL VERIFICATION — may not be available in CI)
    Steps:
      1. xcodebuild -project ios/PhoneCam/PhoneCam.xcodeproj -scheme PhoneCam -sdk iphonesimulator -configuration Debug build 2>&1
      2. Assert: exit code 0
      3. Assert: Build Succeeded in output
    Expected Result: iOS app builds for simulator
    Evidence: Build output captured
    Note: MANUAL VERIFICATION REQUIRED if Xcode not available on agent machine
  ```

  **Commit**: YES
  - Message: `feat(ios): set up iOS project with Rust FFI via UniFFI and raw C bridge`
  - Files: `ios/**`, `rust/phonecam-mobile-core/**` (updated for iOS targets)
  - Pre-commit: `cargo test -p phonecam-mobile-core`

---

- [x] 8. Linux v4l2loopback Driver Integration

  **What to do**:
  - **RED**: Write tests for v4l2loopback device detection, creation, and frame writing
  - Create `phonecam-driver-linux` Rust crate
  - Implement v4l2loopback device detection: check if module is loaded (`/sys/module/v4l2loopback`), enumerate `/dev/video*` devices
  - Implement device creation: use v4l2 ioctls via `v4l` crate or `nix` crate to open loopback device, set format (YUYV or NV12)
  - Implement frame writer: accept raw decoded frames (NV12/YUYV), write to v4l2 device fd with `VIDIOC_QBUF`
  - Implement user guidance: if v4l2loopback not loaded, return structured error with installation instructions per distro
  - Set `exclusive_caps=1` requirement — validate on open, fail with clear error if not set
  - **GREEN + REFACTOR**: Implement and clean up

  **Must NOT do**:
  - Do NOT attempt to install/load v4l2loopback kernel module programmatically (needs sudo)
  - Do NOT implement audio loopback (snd-aloop) — defer to v2

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Low-level Linux systems programming, V4L2 ioctls

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 9-13)
  - **Blocks**: 14
  - **Blocked By**: 5

  **References**:
  - **Crate**: `v4l` — safe V4L2 bindings for Rust: https://docs.rs/v4l
  - **Source**: v4l2loopback kernel module — ioctls and format handling: https://github.com/v4l2loopback/v4l2loopback/blob/master/v4l2loopback.c
  - **Pattern**: DroidCam v4l2 writer — YUYV format, buffer management: https://github.com/dev47apps/droidcam-linux-client/blob/master/src/decoder.c
  - **Docs**: V4L2 API — streaming I/O (MMAP): https://www.kernel.org/doc/html/latest/userspace-api/media/v4l/mmap.html

  **Acceptance Criteria**:
  - [ ] TDD: Tests written FIRST
  - [ ] `cargo test -p phonecam-driver-linux` → PASS
  - [ ] Detects whether v4l2loopback is loaded
  - [ ] Opens v4l2loopback device, sets format to NV12/YUYV at specified resolution
  - [ ] Writes test frames → `v4l2-ctl --device=/dev/videoN --all` shows correct format
  - [ ] `ffprobe -f v4l2 -i /dev/videoN` shows video stream with correct resolution
  - [ ] Returns structured error with installation instructions when v4l2loopback not loaded

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Write test frames to v4l2loopback
    Tool: Bash
    Preconditions: v4l2loopback loaded with exclusive_caps=1
    Steps:
      1. cargo test -p phonecam-driver-linux -- --test-threads=1 2>&1
      2. Assert: exit code 0
      3. After test frame writing: v4l2-ctl --device=/dev/videoN --all 2>&1
      4. Assert: output contains expected resolution (e.g., "1280x720")
      5. timeout 3 ffprobe -f v4l2 -i /dev/videoN 2>&1
      6. Assert: output contains "Video:" with correct dimensions
    Expected Result: Virtual webcam receives frames
    Evidence: v4l2-ctl and ffprobe output captured

  Scenario: v4l2loopback not loaded detection
    Tool: Bash (cargo test)
    Preconditions: v4l2loopback NOT loaded
    Steps:
      1. sudo modprobe -r v4l2loopback 2>/dev/null; cargo test -p phonecam-driver-linux -- not_loaded 2>&1
      2. Assert: test passes
      3. Assert: error message contains installation instructions
    Expected Result: Clean error with guidance
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat(driver-linux): implement v4l2loopback integration with detection and frame writing`
  - Files: `rust/phonecam-driver-linux/src/**`
  - Pre-commit: `cargo test -p phonecam-driver-linux`

---

- [x] 9. Desktop H.264 Decode Pipeline

  **What to do**:
  - **RED**: Write tests for H.264 NAL unit decoding to raw frames
  - Add `ffmpeg-next` crate dependency to `phonecam-desktop`
  - Implement H.264 decoder: accept NAL unit bytes from `phonecam-protocol::VideoFrame`, decode to raw frame (NV12)
  - Implement frame format conversion: NV12 → YUYV (for v4l2loopback) or NV12 → platform-specific format
  - Implement decode pipeline: `VideoFrame → H.264 Decode → Format Convert → Raw Frame Buffer`
  - Handle keyframe requests: if decoder loses sync, send `CameraControl::RequestKeyframe` via transport
  - Benchmark decode latency — target < 20ms per frame for 1080p
  - **GREEN + REFACTOR**: Implement and optimize

  **Must NOT do**:
  - Do NOT implement hardware-accelerated decoding in v1 (software decode via ffmpeg is fine for desktop)
  - Do NOT implement audio decoding (reserved for v2)

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - FFmpeg/video processing, Rust systems programming

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 8, 10-13)
  - **Blocks**: 14, 15, 16, 17
  - **Blocked By**: 5

  **References**:
  - **Crate**: `ffmpeg-next` — Rust FFmpeg bindings: https://docs.rs/ffmpeg-next
  - **Pattern**: scrcpy decoder — FFmpeg H.264 decode pipeline: https://github.com/dev47apps/droidcam-linux-client/blob/master/src/decoder.c
  - **Docs**: FFmpeg decode API — `avcodec_send_packet()` / `avcodec_receive_frame()`: https://ffmpeg.org/doxygen/trunk/group__lavc__decoding.html
  - **Pattern**: NV12 → YUYV conversion using `swscale`: https://ffmpeg.org/doxygen/trunk/group__libsws.html

  **Acceptance Criteria**:
  - [ ] TDD: Tests written FIRST
  - [ ] `cargo test -p phonecam-desktop -- decode` → PASS
  - [ ] Decodes test H.264 file NAL units to raw NV12 frames
  - [ ] Format conversion NV12 → YUYV produces valid output
  - [ ] Decode latency < 20ms per 1080p frame (benchmarked)
  - [ ] Handles missing keyframe gracefully (requests IDR, doesn't crash)

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Decode test H.264 to raw frames
    Tool: Bash (cargo test)
    Preconditions: ffmpeg development libraries installed, test H.264 file available
    Steps:
      1. cargo test -p phonecam-desktop -- decode --test-threads=1 2>&1
      2. Assert: exit code 0
      3. Assert: test "decode_h264_to_nv12" passes
      4. Assert: test "convert_nv12_to_yuyv" passes
      5. Assert: test "decode_latency_benchmark" passes (< 20ms avg)
    Expected Result: H.264 decode pipeline works correctly
    Evidence: Test output with benchmark data
  ```

  **Commit**: YES
  - Message: `feat(desktop): implement H.264 decode pipeline with format conversion`
  - Files: `rust/phonecam-desktop/src/decode.rs`, `rust/phonecam-desktop/src/convert.rs`
  - Pre-commit: `cargo test -p phonecam-desktop`

---

- [x] 10. macOS CMIO Camera Extension (Swift)

  **What to do**:
  - Create Swift Xcode project in `apple/PhoneCamDriver/` as a Camera Extension target
  - Implement `CMIOExtensionProviderSource` protocol — the virtual camera source
  - Implement `CMIOExtensionStreamSource` — provides video frames to consuming apps
  - Create frame buffer management: receive raw frames via IPC, convert to `CMSampleBuffer`
  - Implement IPC receiver: use UNIX socket or CFMessagePort to receive frames from Tauri app
    - App Group ID required for sandboxed IPC
    - Create helper XPC service or named pipe in shared App Group container
  - Register device with system as "PhoneCam" virtual camera
  - Handle System Extension lifecycle: install, enable, update
  - Reference: OBS mac-virtualcam Camera Extension implementation and Halle/SinkCam

  **Must NOT do**:
  - Do NOT use deprecated CMIO DAL plugin API — Camera Extensions only (macOS 12.3+)
  - Do NOT implement audio extension in v1
  - Do NOT try to write this in Rust — must be Swift/Obj-C

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - macOS system programming, Swift, CoreMediaIO framework

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 8, 9, 11-13)
  - **Blocks**: 15
  - **Blocked By**: 2

  **References**:
  - **Pattern**: OBS Studio macOS Camera Extension: https://github.com/obsproject/obs-studio/tree/master/plugins/mac-virtualcam/src/camera-extension
  - **Pattern**: Halle/SinkCam — CMIO Sink Stream example: https://github.com/Halle/SinkCam
  - **Docs**: Apple WWDC22 — Create Camera Extensions with Core Media IO: https://developer.apple.com/videos/play/wwdc2022/10022/
  - **Docs**: Apple CoreMediaIO Camera Extension API: https://developer.apple.com/documentation/coremediaio/creating-a-camera_extension_with_core_media_i_o

  **Acceptance Criteria**:
  - [ ] Camera Extension builds as `.appex` bundle
  - [ ] Extension registers as "PhoneCam" in system camera list
  - [ ] Receiving test frames via IPC and presenting to consumers
  - [ ] FaceTime or Photo Booth shows "PhoneCam" as available camera
  - [ ] **MANUAL VERIFICATION REQUIRED**: macOS with Developer ID signing + SIP enabled needed

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Camera Extension builds
    Tool: Bash
    Preconditions: Xcode installed with macOS SDK
    Steps:
      1. xcodebuild -project apple/PhoneCamDriver/PhoneCamDriver.xcodeproj -scheme PhoneCamDriver -configuration Debug build 2>&1
      2. Assert: exit code 0 (or note: MANUAL VERIFICATION if Xcode unavailable)
    Expected Result: Extension builds successfully
    Evidence: Build output captured
    Note: MANUAL VERIFICATION REQUIRED — Full testing needs macOS with Developer ID
  ```

  **Commit**: YES
  - Message: `feat(driver-macos): implement CMIO Camera Extension with IPC frame receiver`
  - Files: `apple/PhoneCamDriver/**`
  - Pre-commit: `xcodebuild build` (if available)

---

- [x] 11. Windows Virtual Camera Driver (C++)

  **What to do**:
  - Create C++ project in `windows/PhoneCamDriver/` with CMake or MSBuild
  - **Option A (Windows 11+)**: Implement Frame Server Custom Media Source COM DLL
    - Implement `IMFMediaSourceEx`, `IMFMediaStream`, `IKsControl` COM interfaces
    - Create stub driver package for device registration
    - Register under `KSCATEGORY_VIDEO_CAMERA`
  - **Option B (Windows 10 fallback)**: Implement DirectShow source filter
    - Create DirectShow filter DLL that emulates capture device
    - Register with `DllRegisterServer` / `regsvr32`
  - Implement shared memory or named pipe for receiving frames from Tauri app
  - Present as "PhoneCam" in Windows camera device list
  - Reference: smourier/VCamSample for Frame Server, OBS Virtual Camera for DirectShow

  **Must NOT do**:
  - Do NOT attempt this in Rust — must be C++ with COM
  - Do NOT implement audio (virtual microphone) in v1
  - Do NOT implement driver signing for v1 — developer/test mode only

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Windows COM programming, C++, DirectShow/Media Foundation

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 8-10, 12-13)
  - **Blocks**: 16
  - **Blocked By**: 2

  **References**:
  - **Pattern**: smourier/VCamSample — Windows 11 Frame Server virtual camera: https://github.com/smourier/VCamSample
  - **Pattern**: OBS Virtual Camera DirectShow filter: https://github.com/Fenrirthviti/obs-virtual-cam
  - **Docs**: Microsoft Frame Server Custom Media Source: https://learn.microsoft.com/en-us/windows-hardware/drivers/stream/frame-server-custom-media-source
  - **Pattern**: roman380/tmhare.mvps.org-vcam — DirectShow source filter: https://github.com/roman380/tmhare.mvps.org-vcam

  **Acceptance Criteria**:
  - [ ] C++ project builds with CMake or MSBuild
  - [ ] COM DLL (or DirectShow filter) registers successfully
  - [ ] "PhoneCam" appears in Windows camera device list
  - [ ] Shared memory / named pipe receiver works for frame data
  - [ ] **MANUAL VERIFICATION REQUIRED**: Windows machine with test signing mode needed

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Windows driver project builds
    Tool: Bash
    Preconditions: MSVC or MinGW toolchain (MANUAL VERIFICATION — likely Windows only)
    Steps:
      1. cmake -B build windows/PhoneCamDriver 2>&1
      2. cmake --build build 2>&1
      3. Assert: exit code 0
      4. Assert: DLL output exists
    Expected Result: Driver DLL builds
    Evidence: Build output captured
    Note: MANUAL VERIFICATION REQUIRED — Full testing needs Windows with test signing
  ```

  **Commit**: YES
  - Message: `feat(driver-windows): implement virtual camera COM DLL with frame receiver`
  - Files: `windows/PhoneCamDriver/**`
  - Pre-commit: `cmake --build build`

---

- [x] 12. Android Camera Capture + H.264 Encoding

  **What to do**:
  - Implement camera capture using CameraX (Jetpack) in Android app
  - Configure `ImageAnalysis` use case for frame access, or `Preview` + surface-based capture
  - Implement H.264 hardware encoding using `MediaCodec.createEncoderByType("video/avc")`
    - Configure: Baseline profile, no B-frames, I-frame interval 1 second, bitrate 3-5 Mbps
    - Enable low-latency mode where available (Android 11+)
  - Pass encoded H.264 NAL units to Rust core via raw `extern "C"` FFI: `phonecam_send_video_frame(data, len, pts, is_keyframe)`
  - Build Rust transport client in `phonecam-mobile-core`: connect to desktop, send video frames
  - Implement Android UI: camera preview (SurfaceView), connection status, basic controls
  - Support configurable resolution: 480p, 720p, 1080p and FPS: 15, 30, 60 (based on device capabilities)
  - Query available cameras and resolutions using CameraManager

  **Must NOT do**:
  - Do NOT use software encoding (x264/openh264) — hardware only via MediaCodec
  - Do NOT implement audio capture yet (protocol reserves it, but implementation deferred)
  - Do NOT implement USB/ADB connection yet (TODO 18)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
    - Android development, CameraX, MediaCodec

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 8-11, 13)
  - **Blocks**: 14, 18
  - **Blocked By**: 6

  **References**:
  - **Docs**: Android CameraX architecture + video call stream use case: https://developer.android.com/media/camera/camerax/architecture
  - **Docs**: Android MediaCodec for H.264 encoding: https://developer.android.com/reference/android/media/MediaCodec
  - **Pattern**: scrcpy camera capture server (Java) — Camera2 + MediaCodec: https://github.com/Genymobile/scrcpy/blob/master/server/src/main/java/com/genymobile/scrcpy/
  - **Docs**: CameraX supported resolutions: https://developer.android.com/media/camera/camerax/configuration

  **Acceptance Criteria**:
  - [ ] Camera preview displays in app
  - [ ] MediaCodec encoder produces H.264 NAL units
  - [ ] Encoded frames pass through Rust FFI without corruption
  - [ ] Configurable resolution (at least 720p and 1080p)
  - [ ] App connects to desktop `phonecam-transport` server and sends frames
  - [ ] `./gradlew assembleDebug` builds successfully

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Android app captures and encodes
    Tool: Bash (ADB with emulator)
    Preconditions: Android emulator running with camera support
    Steps:
      1. adb install android/app/build/outputs/apk/debug/app-debug.apk 2>&1
      2. adb shell am start -n com.phonecam.app/.MainActivity 2>&1
      3. Assert: Activity started
      4. adb logcat -t 100 | grep "PhoneCam" 2>&1
      5. Assert: log contains "Camera opened" or "Encoder started"
    Expected Result: App starts, camera initializes, encoder produces frames
    Evidence: Logcat output captured
    Note: Camera emulation in Android emulator may produce synthetic frames only
  ```

  **Commit**: YES
  - Message: `feat(android): implement CameraX capture with MediaCodec H.264 encoding and Rust FFI`
  - Files: `android/app/src/**`
  - Pre-commit: `./gradlew assembleDebug`

---

- [x] 13. iOS Camera Capture + H.264 Encoding

  **What to do**:
  - Implement camera capture using AVFoundation in iOS app
  - Configure `AVCaptureSession` with `AVCaptureVideoDataOutput` for frame-by-frame access
  - Implement H.264 hardware encoding using VideoToolbox `VTCompressionSessionCreate()`
    - Configure: Baseline profile, no B-frames, allow frame reordering = false, bitrate 3-5 Mbps
    - Set `kVTCompressionPropertyKey_RealTime = true` for low-latency
  - Pass encoded H.264 NAL units to Rust core via raw C FFI
  - Build Rust transport client integration: connect to desktop, send video frames
  - Implement iOS UI: camera preview (AVCaptureVideoPreviewLayer), connection status
  - Support configurable resolution and FPS (query device capabilities)
  - Handle iOS-specific: camera interruption (phone call), background restrictions

  **Must NOT do**:
  - Do NOT implement USB support (WiFi only for iOS v1)
  - Do NOT implement audio capture
  - Do NOT use software encoding

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []
    - iOS development, AVFoundation, VideoToolbox

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 3 (with TODOs 8-12)
  - **Blocks**: 17
  - **Blocked By**: 7

  **References**:
  - **Docs**: AVFoundation capture setup: https://developer.apple.com/documentation/avfoundation/capture_setup
  - **Docs**: VideoToolbox compression: https://developer.apple.com/documentation/videotoolbox/vtcompressionsession
  - **Pattern**: Camo SDK — AVFoundation for webcam streaming: https://reincubate.com/support/camo-sdk/overview-camo-sdk
  - **Docs**: Handling interruptions: https://developer.apple.com/documentation/avfoundation/avcapturesession/1390location

  **Acceptance Criteria**:
  - [ ] Camera preview displays in app
  - [ ] VideoToolbox encoder produces H.264 NAL units
  - [ ] Encoded frames pass through Rust C FFI without corruption
  - [ ] Configurable resolution and FPS
  - [ ] App connects to desktop and streams frames over WiFi
  - [ ] Camera interruption handled gracefully (shows "paused" state)
  - [ ] Xcode project builds for iOS device/simulator

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: iOS app builds and basic structure
    Tool: Bash
    Preconditions: Xcode installed (MANUAL VERIFICATION)
    Steps:
      1. xcodebuild -project ios/PhoneCam/PhoneCam.xcodeproj -scheme PhoneCam -sdk iphonesimulator build 2>&1
      2. Assert: exit code 0
    Expected Result: iOS app builds for simulator
    Evidence: Build output captured
    Note: MANUAL VERIFICATION REQUIRED — Real camera testing requires physical device
  ```

  **Commit**: YES
  - Message: `feat(ios): implement AVFoundation capture with VideoToolbox H.264 encoding and Rust FFI`
  - Files: `ios/PhoneCam/**`
  - Pre-commit: `xcodebuild build -sdk iphonesimulator`

---

- [x] 14. Linux + Android WiFi End-to-End Integration

  **What to do**:
  - Wire the complete pipeline: Android phone (CameraX → MediaCodec → Rust transport) → WiFi → Linux desktop (Rust transport → H.264 decode → v4l2loopback)
  - Implement the main streaming loop in desktop Tauri backend:
    1. Start mDNS advertisement
    2. Listen for phone connection via `phonecam-transport`
    3. On connection: receive `VideoFrame` messages
    4. Decode H.264 via ffmpeg-next
    5. Convert to YUYV
    6. Write to v4l2loopback device
  - Test with real Android device or emulator → verify frames appear in `ffprobe`
  - Measure end-to-end latency: target ≤ 70ms over WiFi
  - Handle disconnect: stop decode pipeline, close v4l2 device, show "disconnected" in UI

  **Must NOT do**:
  - Do NOT implement auto-reconnection
  - Do NOT implement USB (separate TODO 18)
  - Do NOT optimize for edge cases yet (that's TODO 22)

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Full-stack integration, async Rust, video pipeline

  **Parallelization**:
  - **Can Run In Parallel**: YES (partially)
  - **Parallel Group**: Wave 4 (with TODOs 15-19)
  - **Blocks**: 20, 21, 22
  - **Blocked By**: 8, 9, 12

  **References**:
  - **All previous crates**: phonecam-protocol, phonecam-transport, phonecam-driver-linux, phonecam-desktop
  - **Pattern**: scrcpy E2E pipeline: capture → encode → transport → decode → display/v4l2
  - **Crate**: `tokio::select!` for multiplexing receive + UI events

  **Acceptance Criteria**:
  - [ ] Android app connects to Linux desktop over WiFi
  - [ ] Frames appear on v4l2loopback device
  - [ ] `ffprobe -f v4l2 -i /dev/videoN` shows video stream with correct resolution/fps
  - [ ] `ffmpeg -f v4l2 -i /dev/videoN -frames:v 30 -f null -` succeeds (captures 30 frames)
  - [ ] End-to-end latency ≤ 100ms on LAN (measured with timestamp comparison)
  - [ ] Disconnect from phone → desktop shows "disconnected" status
  - [ ] All `cargo test --workspace` still pass

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: End-to-end WiFi streaming
    Tool: Bash
    Preconditions: v4l2loopback loaded, Android device/emulator available, same network
    Steps:
      1. Start desktop app: cargo run -p phonecam-desktop &
      2. Connect Android app (or send test stream from another process)
      3. Wait 5 seconds for connection establishment
      4. v4l2-ctl --device=/dev/videoN --all 2>&1
      5. Assert: shows "PhoneCam" device name and correct format
      6. timeout 5 ffmpeg -f v4l2 -i /dev/videoN -frames:v 30 -f null - 2>&1
      7. Assert: "frame=30" in output (captured 30 frames)
      8. ffprobe -f v4l2 -i /dev/videoN 2>&1
      9. Assert: shows "Video:" with correct resolution
    Expected Result: Full pipeline works, frames captured from virtual webcam
    Evidence: ffprobe and ffmpeg output captured
  ```

  **Commit**: YES
  - Message: `feat: wire Linux + Android WiFi end-to-end streaming pipeline`
  - Files: `rust/phonecam-desktop/src/pipeline.rs`, integration changes
  - Pre-commit: `cargo test --workspace`

---

- [x] 15. macOS Desktop Integration (Tauri + CMIO IPC)

  **What to do**:
  - Implement IPC bridge from Tauri Rust backend to CMIO Camera Extension
  - Set up App Group container for shared IPC (UNIX socket or CFMessagePort)
  - Implement frame delivery: decoded raw frames → IPC channel → CMIO Extension → system camera
  - Integrate streaming pipeline: transport receive → H.264 decode → format convert → IPC send → CMIO Extension
  - Implement CMIO Extension installation/activation workflow in Tauri
  - Build macOS-specific Tauri bundle including Camera Extension `.appex`
  - Test with FaceTime or Photo Booth showing "PhoneCam"

  **Must NOT do**:
  - Do NOT modify the CMIO Extension (built in TODO 10)
  - Do NOT implement audio

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - macOS system programming, IPC, Tauri

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with TODOs 14, 16-19)
  - **Blocks**: 20, 22
  - **Blocked By**: 5, 9, 10

  **References**:
  - **Pattern**: OBS macOS virtual camera IPC (Mach IPC / CMSampleBuffer): https://github.com/obsproject/obs-studio/blob/master/plugins/mac-virtualcam/
  - **Pattern**: SinkCam CMIO Sink stream for frame delivery: https://github.com/Halle/SinkCam
  - **Docs**: App Groups for IPC: https://developer.apple.com/documentation/security/app-sandbox

  **Acceptance Criteria**:
  - [ ] IPC channel established between Tauri app and CMIO Extension
  - [ ] Frames flow through IPC to Camera Extension
  - [ ] "PhoneCam" appears in FaceTime camera selection
  - [ ] **MANUAL VERIFICATION REQUIRED**: macOS with code signing needed

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: macOS integration
    Tool: Bash
    Note: MANUAL VERIFICATION REQUIRED — Full testing needs macOS with Developer ID
    Steps:
      1. cargo tauri build --target aarch64-apple-darwin 2>&1 (if on macOS)
      2. Assert: build succeeds
    Expected Result: macOS app bundle builds
    Evidence: Build output captured
  ```

  **Commit**: YES
  - Message: `feat(desktop): integrate macOS CMIO Camera Extension IPC and streaming pipeline`
  - Files: `rust/phonecam-desktop/src/driver_macos.rs`, macOS-specific code
  - Pre-commit: `cargo test --workspace`

---

- [x] 16. Windows Desktop Integration (Tauri + COM Driver)

  **What to do**:
  - Implement IPC bridge from Tauri Rust backend to Windows virtual camera COM DLL
  - Use named pipe or shared memory for frame delivery
  - Implement frame delivery: decoded raw frames → shared memory/pipe → COM DLL → system camera
  - Integrate streaming pipeline on Windows
  - Implement driver registration workflow: `regsvr32` for DirectShow or device registration for Frame Server
  - Build Windows-specific Tauri installer (`.msi` or `.exe`) including driver DLL
  - Test with Windows Camera app or OBS showing "PhoneCam"

  **Must NOT do**:
  - Do NOT implement driver signing (test mode only in v1)
  - Do NOT implement audio

  **Recommended Agent Profile**:
  - **Category**: `ultrabrain`
  - **Skills**: []
    - Windows COM, IPC, Tauri

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with TODOs 14-15, 17-19)
  - **Blocks**: 20, 22
  - **Blocked By**: 5, 9, 11

  **References**:
  - **Pattern**: smourier/VCamSample — Frame delivery IPC: https://github.com/smourier/VCamSample
  - **Pattern**: OBS Virtual Camera DirectShow: https://github.com/Fenrirthviti/obs-virtual-cam
  - **Docs**: Windows named pipes: https://learn.microsoft.com/en-us/windows/win32/ipc/named-pipes

  **Acceptance Criteria**:
  - [ ] Frame delivery works via shared memory/named pipe
  - [ ] "PhoneCam" appears in Windows camera device list
  - [ ] OBS detects "PhoneCam" as video capture device
  - [ ] **MANUAL VERIFICATION REQUIRED**: Windows machine with test signing needed

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Windows integration
    Tool: Bash
    Note: MANUAL VERIFICATION REQUIRED — Full testing needs Windows
    Steps:
      1. cargo tauri build --target x86_64-pc-windows-msvc 2>&1 (if on Windows)
      2. Assert: build succeeds
    Expected Result: Windows installer builds
    Evidence: Build output captured
  ```

  **Commit**: YES
  - Message: `feat(desktop): integrate Windows virtual camera COM driver and streaming pipeline`
  - Files: `rust/phonecam-desktop/src/driver_windows.rs`, Windows-specific code
  - Pre-commit: `cargo test --workspace`

---

- [x] 17. iOS WiFi Integration

  **What to do**:
  - Wire iOS app to connect to desktop over WiFi using Rust transport client
  - Implement connection flow: discover desktop via mDNS → connect → stream H.264
  - Integrate camera capture (TODO 13) with transport: frames → encode → Rust FFI → TCP → desktop
  - Test E2E: iOS app → WiFi → desktop → virtual webcam
  - Handle iOS-specific: background state (pause streaming), app lifecycle

  **Must NOT do**:
  - Do NOT implement USB (WiFi only for iOS v1)
  - Do NOT implement background streaming (iOS doesn't fully support it)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with TODOs 14-16, 18-19)
  - **Blocks**: 22
  - **Blocked By**: 5, 9, 13

  **References**:
  - **Crate**: phonecam-mobile-core (shared with Android)
  - **Docs**: iOS Network framework for local network access: https://developer.apple.com/documentation/network

  **Acceptance Criteria**:
  - [ ] iOS app discovers desktop via mDNS
  - [ ] iOS app connects and streams H.264 over WiFi
  - [ ] Desktop receives frames and pushes to virtual webcam
  - [ ] **MANUAL VERIFICATION REQUIRED**: iOS device/simulator + macOS needed

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: iOS WiFi streaming
    Note: MANUAL VERIFICATION REQUIRED — Requires iOS device and macOS
  ```

  **Commit**: YES
  - Message: `feat(ios): integrate WiFi streaming with desktop via Rust transport`
  - Files: `ios/PhoneCam/**`
  - Pre-commit: `xcodebuild build`

---

- [ ] 18. Android USB (ADB) Integration

  **What to do**:
  - Implement ADB port forwarding in desktop Tauri backend
  - Detect connected Android devices: run `adb devices` and parse output
  - Forward local port to Android device: `adb forward tcp:LOCAL_PORT tcp:DEVICE_PORT`
  - Connect transport through forwarded port (same TCP protocol, just over USB tunnel)
  - Implement USB connection option in Tauri UI: "Connect via USB" button
  - Handle ADB lifecycle: start ADB daemon if needed, detect device connect/disconnect
  - Bundle `adb` binary or detect system-installed `adb`
  - Implement user onboarding: guide to enable USB Debugging on Android

  **Must NOT do**:
  - Do NOT implement custom USB protocol (ADB tunnel is sufficient)
  - Do NOT implement iOS USB (deferred)
  - Do NOT implement `libusb` direct access (too much scope)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with TODOs 14-17, 19)
  - **Blocks**: 22
  - **Blocked By**: 6, 12

  **References**:
  - **Pattern**: scrcpy ADB forwarding: https://github.com/Genymobile/scrcpy/blob/master/app/src/adb/adb.c
  - **Docs**: ADB port forwarding: https://developer.android.com/tools/adb#forwardports
  - **Tool**: Android platform-tools (contains `adb`): https://developer.android.com/tools/releases/platform-tools
  - **License**: Android platform-tools are Apache-2.0 — safe to bundle

  **Acceptance Criteria**:
  - [ ] Desktop detects connected Android device via `adb devices`
  - [ ] Port forwarding established: `adb forward tcp:PORT tcp:PORT`
  - [ ] Video streaming works over USB (same protocol, lower latency)
  - [ ] End-to-end latency ≤ 70ms over USB
  - [ ] UI shows "USB Connected" when Android device plugged in
  - [ ] Works without user-installed ADB (bundled or guided installation)

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: ADB port forwarding
    Tool: Bash
    Preconditions: Android device with USB Debugging enabled, connected via USB
    Steps:
      1. adb devices 2>&1
      2. Assert: device listed
      3. adb forward tcp:8080 tcp:8080 2>&1
      4. Assert: exit code 0
      5. Start desktop streaming receiver
      6. Start Android app
      7. timeout 5 ffmpeg -f v4l2 -i /dev/videoN -frames:v 30 -f null - 2>&1
      8. Assert: "frame=30" in output
    Expected Result: Streaming works over USB with ADB forwarding
    Evidence: ADB and ffmpeg output captured
    Note: Requires physical Android device for full verification
  ```

  **Commit**: YES
  - Message: `feat(desktop): implement ADB port forwarding for Android USB streaming`
  - Files: `rust/phonecam-desktop/src/adb.rs`, UI updates
  - Pre-commit: `cargo test --workspace`

---

- [x] 19. mDNS Discovery + QR Code Fallback Integration

  **What to do**:
  - Integrate `phonecam-discovery` into Tauri desktop (publish service on startup)
  - Integrate `phonecam-discovery` into Android and iOS apps (browse for services)
  - Implement discovered device list in mobile UI: show available desktops with names
  - Implement QR code generation in Tauri desktop: display QR code with `phonecam://IP:PORT?name=NAME`
  - Implement QR code scanning in mobile apps: scan QR → parse URI → connect
  - Handle network changes: re-publish on IP change, re-browse on WiFi switch
  - Use `qrcode` Rust crate for QR generation in Tauri, native QR scanner libs for mobile

  **Must NOT do**:
  - Do NOT implement cloud-based discovery
  - Do NOT implement Bluetooth discovery

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: [`frontend-ui-ux`]
    - `frontend-ui-ux`: QR code display UI in Tauri, device list UI in mobile

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 4 (with TODOs 14-18)
  - **Blocks**: 22
  - **Blocked By**: 4, 5, 6, 7

  **References**:
  - **Crate**: phonecam-discovery (TODO 4)
  - **Crate**: `qrcode` for QR generation: https://docs.rs/qrcode
  - **Android**: ML Kit Barcode Scanning for QR: https://developers.google.com/ml-kit/vision/barcode-scanning
  - **iOS**: AVFoundation `AVCaptureMetadataOutput` for QR: https://developer.apple.com/documentation/avfoundation/avcapturemetadataoutput

  **Acceptance Criteria**:
  - [ ] Desktop publishes mDNS service on startup
  - [ ] Mobile app discovers desktop within 3 seconds on same subnet
  - [ ] Tapping discovered device connects and starts streaming
  - [ ] Desktop shows QR code in UI
  - [ ] Mobile app scans QR code and connects successfully
  - [ ] QR code fallback works when mDNS fails (different subnets)

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: mDNS discovery integration
    Tool: Bash (cargo test + integration)
    Steps:
      1. cargo test --workspace -- discovery 2>&1
      2. Assert: all discovery tests pass
      3. Start desktop app
      4. Assert: mDNS service published (avahi-browse -a | grep phonecam)
    Expected Result: Service published and discoverable
    Evidence: avahi-browse output captured

  Scenario: QR code generation
    Tool: Playwright
    Preconditions: Tauri app running
    Steps:
      1. Navigate to app
      2. Click "Show QR Code" button
      3. Assert: QR code image element visible
      4. Screenshot: .sisyphus/evidence/task-19-qr-code.png
    Expected Result: QR code displayed in Tauri UI
    Evidence: .sisyphus/evidence/task-19-qr-code.png
  ```

  **Commit**: YES
  - Message: `feat: integrate mDNS discovery and QR code fallback across all platforms`
  - Files: `rust/phonecam-desktop/src/discovery.rs`, mobile app discovery code
  - Pre-commit: `cargo test --workspace`

---

- [x] 20. Tauri Settings Frontend (Resolution/FPS/Camera Controls UI)

  **What to do**:
  - Build settings panel in Tauri web frontend
  - Resolution selector: dropdown with 480p, 720p, 1080p options
  - FPS selector: dropdown with 15, 30, 60 options
  - Camera info display: currently connected device name, camera (front/back), connection type (WiFi/USB)
  - Connection controls: Connect, Disconnect buttons
  - Device list: discovered devices via mDNS, manual IP entry fallback
  - Status indicators: connection state, FPS counter, latency display
  - Settings persistence: save last-used settings to local storage
  - Responsive layout that works on all desktop sizes

  **Must NOT do**:
  - Do NOT implement advanced camera controls (zoom, exposure, WB — v2)
  - Do NOT implement themes/dark mode (nice-to-have, not v1)
  - Do NOT use heavy frontend frameworks — keep it lightweight (vanilla JS or minimal framework)

  **Recommended Agent Profile**:
  - **Category**: `visual-engineering`
  - **Skills**: [`frontend-ui-ux`]
    - `frontend-ui-ux`: Settings panel design, responsive layout

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with TODOs 21, 22)
  - **Blocks**: 22
  - **Blocked By**: 14

  **References**:
  - **Docs**: Tauri v2 frontend: https://v2.tauri.app/develop/
  - **Pattern**: DroidCam desktop UI — minimal settings panel reference
  - **Docs**: Tauri invoke commands from frontend: https://v2.tauri.app/develop/calling-rust/

  **Acceptance Criteria**:
  - [ ] Resolution dropdown shows 480p, 720p, 1080p
  - [ ] FPS dropdown shows 15, 30, 60
  - [ ] Device list populated from mDNS discovery
  - [ ] Connect/Disconnect buttons work
  - [ ] Status shows: connected/disconnected, current FPS, latency
  - [ ] Settings persist across app restarts

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Settings UI functional test
    Tool: Playwright (playwright skill)
    Preconditions: Tauri app running via cargo tauri dev
    Steps:
      1. Navigate to app
      2. Assert: select[name="resolution"] is visible with 3 options
      3. Select "1080p" from resolution dropdown
      4. Assert: select[name="fps"] is visible with 3 options
      5. Select "30" from FPS dropdown
      6. Assert: "Not Connected" status indicator visible
      7. Assert: "Discovered Devices" section visible
      8. Assert: manual IP input field exists
      9. Screenshot: .sisyphus/evidence/task-20-settings-ui.png
    Expected Result: Full settings UI renders correctly
    Evidence: .sisyphus/evidence/task-20-settings-ui.png

  Scenario: Settings persistence
    Tool: Playwright
    Steps:
      1. Select "1080p" resolution, "60" FPS
      2. Close app, reopen
      3. Assert: resolution shows "1080p", FPS shows "60"
    Expected Result: Settings persist
    Evidence: Screenshot after reopen
  ```

  **Commit**: YES
  - Message: `feat(desktop): build settings frontend with resolution/FPS controls and device list`
  - Files: `rust/phonecam-desktop/src-tauri/`, frontend files
  - Pre-commit: `cargo test -p phonecam-desktop`

---

- [x] 21. Camera Front/Back Switching Protocol + Implementation

  **What to do**:
  - Implement `CameraControl::SwitchCamera { front: bool }` message handling in transport layer
  - Desktop UI: "Switch Camera" button that sends control message to phone
  - Mobile app: receive control message → switch between front and back camera
  - Handle camera switch gracefully:
    1. Pause stream
    2. Close current camera session
    3. Open new camera (front/back)
    4. Reconfigure encoder
    5. Send keyframe (IDR) to desktop
    6. Resume stream
  - Handle orientation: lock to current orientation during switch
  - Android: use CameraManager to enumerate and switch
  - iOS: switch AVCaptureDevice input

  **Must NOT do**:
  - Do NOT implement mid-stream resolution change (resolution stays same across switch)
  - Do NOT implement zoom/exposure/WB controls (v2)

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES
  - **Parallel Group**: Wave 5 (with TODOs 20, 22)
  - **Blocks**: 22
  - **Blocked By**: 14

  **References**:
  - **Crate**: phonecam-protocol `CameraControl` message type (TODO 2)
  - **Docs**: Android CameraManager camera selection: https://developer.android.com/reference/android/hardware/camera2/CameraManager
  - **Docs**: iOS AVCaptureDevice switching: https://developer.apple.com/documentation/avfoundation/avcapturedevice

  **Acceptance Criteria**:
  - [ ] Desktop "Switch Camera" button sends control message
  - [ ] Android switches camera within 2 seconds
  - [ ] iOS switches camera within 2 seconds
  - [ ] Stream resumes with keyframe after switch (no corruption)
  - [ ] No disconnect during switch

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Camera switch via protocol
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --workspace -- camera_switch 2>&1
      2. Assert: exit code 0
      3. Assert: test "switch_camera_message_roundtrip" passes
      4. Assert: test "switch_camera_keyframe_sent" passes
    Expected Result: Camera switch protocol works
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat: implement camera front/back switching via control protocol`
  - Files: Protocol, desktop, and mobile changes
  - Pre-commit: `cargo test --workspace`

---

- [ ] 22. Error Handling + Connection State Management

  **What to do**:
  - Implement comprehensive error handling across the pipeline:
    - Network errors: timeout, connection refused, connection lost
    - Camera errors: camera in use by another app, camera disconnected
    - Driver errors: v4l2loopback not loaded, CMIO extension not installed, COM DLL not registered
    - Encoding errors: encoder failure, unsupported format
  - Implement connection state display in both desktop and mobile UIs
  - Desktop states: `Waiting for Connection → Connected → Streaming → Disconnected → Error`
  - Mobile states: `Discovering → Connecting → Streaming → Disconnected → Error`
  - Implement error recovery guidance: clear error messages with suggested actions
  - Lock phone orientation during streaming (prevent mid-stream resolution changes)
  - Handle edge cases:
    - Phone screen lock during streaming → pause and show warning
    - WiFi network change → disconnect cleanly
    - Multiple apps accessing virtual webcam simultaneously (supported on Linux, verify others)

  **Must NOT do**:
  - Do NOT implement auto-reconnection (manual reconnect only in v1)
  - Do NOT implement bandwidth adaptation (fixed quality in v1)
  - Do NOT implement crash recovery/crash reporting

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: []

  **Parallelization**:
  - **Can Run In Parallel**: YES (partially)
  - **Parallel Group**: Wave 5 (with TODOs 20, 21)
  - **Blocks**: 23
  - **Blocked By**: 14-19

  **References**:
  - **Pattern**: Rust error handling with `thiserror` and `anyhow`: https://docs.rs/thiserror
  - **Crate**: `phonecam-transport` connection state machine (TODO 3)

  **Acceptance Criteria**:
  - [ ] All error types have user-friendly messages
  - [ ] Desktop UI shows correct state at all times
  - [ ] Mobile UI shows correct state at all times
  - [ ] v4l2loopback-not-loaded shows installation instructions (Linux)
  - [ ] Connection drop shows "Disconnected" within 5 seconds
  - [ ] `cargo test --workspace` passes with error handling tests

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: Error states display correctly
    Tool: Playwright (playwright skill)
    Preconditions: Tauri app running, no phone connected
    Steps:
      1. Navigate to app
      2. Assert: status shows "Waiting for Connection" or "Not Connected"
      3. Attempt to connect to invalid IP (e.g., 192.168.0.254:9999)
      4. Wait for timeout (5-10s)
      5. Assert: status shows error message with suggestion
      6. Screenshot: .sisyphus/evidence/task-22-error-state.png
    Expected Result: Clear error display with guidance
    Evidence: .sisyphus/evidence/task-22-error-state.png

  Scenario: Error handling in transport
    Tool: Bash (cargo test)
    Steps:
      1. cargo test --workspace -- error_handling 2>&1
      2. Assert: tests for timeout, connection_refused, connection_lost pass
    Expected Result: All error paths tested
    Evidence: Test output captured
  ```

  **Commit**: YES
  - Message: `feat: implement comprehensive error handling and connection state management`
  - Files: Changes across all Rust crates and frontend
  - Pre-commit: `cargo test --workspace`

---

- [ ] 23. Cross-Platform CI/CD Pipeline

  **What to do**:
  - Extend `.github/workflows/ci.yml` to build and test on all platforms:
    - Linux: `cargo test --workspace`, v4l2loopback integration tests
    - macOS: `cargo test --workspace`, Xcode build for Camera Extension and iOS app
    - Windows: `cargo test --workspace`, MSBuild/CMake for COM driver
  - Add Android build to CI: `./gradlew assembleDebug`
  - Add iOS build to CI: `xcodebuild -sdk iphonesimulator build`
  - Set up matrix builds for Rust: `ubuntu-latest`, `macos-latest`, `windows-latest`
  - Add release workflow: build Tauri installers for all platforms on tag push
  - Add code coverage reporting via `cargo tarpaulin` or `cargo llvm-cov`
  - Ensure all `cargo clippy` warnings are errors in CI

  **Must NOT do**:
  - Do NOT set up App Store / Play Store submission (v1 = developer builds)
  - Do NOT implement auto-updates
  - Do NOT implement driver signing in CI

  **Recommended Agent Profile**:
  - **Category**: `unspecified-high`
  - **Skills**: [`git-master`]
    - `git-master`: CI/CD workflow creation

  **Parallelization**:
  - **Can Run In Parallel**: NO
  - **Parallel Group**: Wave 5 (final)
  - **Blocks**: None (final task)
  - **Blocked By**: 22

  **References**:
  - **Pattern**: Tauri CI/CD: https://v2.tauri.app/distribute/ci-cd/
  - **Pattern**: Firezone multi-platform CI: https://github.com/firezone/firezone/.github/workflows
  - **Docs**: GitHub Actions matrix strategy: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/running-variations-of-jobs-in-a-workflow

  **Acceptance Criteria**:
  - [ ] CI runs on push to main and on PRs
  - [ ] Linux: `cargo test --workspace` passes in CI
  - [ ] macOS: `cargo test --workspace` + Xcode build passes
  - [ ] Windows: `cargo test --workspace` + CMake build passes
  - [ ] Android: `./gradlew assembleDebug` passes in CI
  - [ ] iOS: `xcodebuild` simulator build passes in CI
  - [ ] `cargo clippy --workspace -- -D warnings` passes (warnings are errors)
  - [ ] Code coverage reported

  **Agent-Executed QA Scenarios**:
  ```
  Scenario: CI pipeline validates
    Tool: Bash
    Steps:
      1. cat .github/workflows/ci.yml
      2. Assert: file contains jobs for ubuntu, macos, windows
      3. Assert: contains cargo test, clippy, fmt
      4. Assert: contains android build step
      5. Assert: contains ios build step (xcodebuild)
    Expected Result: CI workflow covers all platforms
    Evidence: Workflow file content
  ```

  **Commit**: YES
  - Message: `chore: set up cross-platform CI/CD with matrix builds and release workflow`
  - Files: `.github/workflows/*.yml`
  - Pre-commit: `cargo test --workspace`

---

## Commit Strategy

| After Task | Message | Key Files | Verification |
|-----------|---------|-----------|-------------|
| 1 | `chore: initialize monorepo with Cargo workspace and CI` | Cargo.toml, CI | `cargo test --workspace` |
| 2 | `feat(protocol): implement wire format with H.264 NAL framing` | phonecam-protocol/ | `cargo test -p phonecam-protocol` |
| 3 | `feat(transport): implement TCP transport with connection state machine` | phonecam-transport/ | `cargo test -p phonecam-transport` |
| 4 | `feat(discovery): implement mDNS discovery and QR URI format` | phonecam-discovery/ | `cargo test -p phonecam-discovery` |
| 5 | `feat(desktop): create Tauri app skeleton` | phonecam-desktop/ | `cargo tauri dev` |
| 6 | `feat(android): set up project with Rust FFI` | android/ | `./gradlew assembleDebug` |
| 7 | `feat(ios): set up project with Rust FFI` | ios/ | `xcodebuild build` |
| 8 | `feat(driver-linux): v4l2loopback integration` | phonecam-driver-linux/ | `cargo test` + ffprobe |
| 9 | `feat(desktop): H.264 decode pipeline` | phonecam-desktop/decode | `cargo test` |
| 10 | `feat(driver-macos): CMIO Camera Extension` | apple/ | `xcodebuild build` |
| 11 | `feat(driver-windows): virtual camera COM DLL` | windows/ | `cmake --build` |
| 12 | `feat(android): CameraX + MediaCodec encoding` | android/app/ | `./gradlew assembleDebug` |
| 13 | `feat(ios): AVFoundation + VideoToolbox encoding` | ios/ | `xcodebuild build` |
| 14 | `feat: Linux + Android WiFi E2E pipeline` | integration | `cargo test` + ffprobe |
| 15 | `feat(desktop): macOS CMIO IPC integration` | macOS-specific | `cargo test` |
| 16 | `feat(desktop): Windows COM driver integration` | Windows-specific | `cargo test` |
| 17 | `feat(ios): WiFi streaming integration` | ios/ | `xcodebuild build` |
| 18 | `feat(desktop): ADB port forwarding for Android USB` | adb.rs | `cargo test` |
| 19 | `feat: mDNS discovery + QR code fallback` | discovery integration | `cargo test` |
| 20 | `feat(desktop): settings frontend with controls` | Tauri frontend | Playwright |
| 21 | `feat: camera front/back switching` | protocol + mobile | `cargo test` |
| 22 | `feat: error handling and state management` | all crates | `cargo test --workspace` |
| 23 | `chore: cross-platform CI/CD pipeline` | .github/ | CI passes |

---

## Success Criteria

### Verification Commands
```bash
# All Rust tests pass
cargo test --workspace  # Expected: all tests pass

# Linux virtual webcam works
v4l2-ctl --device=/dev/videoN --all  # Expected: shows "PhoneCam" with correct format
ffprobe -f v4l2 -i /dev/videoN  # Expected: Video stream with correct res/fps

# Android builds
cd android && ./gradlew assembleDebug  # Expected: APK produced

# Desktop builds
cargo tauri build  # Expected: installer for current platform

# CI passes
# Push to main → all matrix jobs green
```

### Final Checklist
- [ ] All "Must Have" requirements present
- [ ] All "Must NOT Have" guardrails respected
- [ ] `cargo test --workspace` passes with 0 failures
- [ ] Android APK builds and installs
- [ ] iOS app builds for simulator
- [ ] Tauri desktop app builds on Linux, macOS, Windows
- [ ] Virtual webcam appears in system camera list on all desktop platforms
- [ ] End-to-end streaming works: phone → desktop → virtual webcam → ffprobe
- [ ] mDNS discovery works on same subnet
- [ ] QR code fallback works
- [ ] Android USB (ADB) streaming works
- [ ] Front/back camera switch works mid-stream
- [ ] All error states display clear messages with guidance
- [ ] CI/CD pipeline passes on all platforms
- [ ] Apache 2.0 license applied to all source files

# Release status

Snapshot: 2026-08-30

PhoneCam is at **core MVP implementation complete / native packaging and hardware qualification pending**. The earlier project plan marked hardware and CI outcomes complete without evidence; those checkboxes have been reopened.

## Verified in this workspace

| Gate | Result |
| --- | --- |
| Rust formatting | Pass: `cargo fmt --all --check` |
| Rust linting | Pass: `cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Rust tests | Pass: 43 unit/integration tests plus doc tests |
| Linux desktop compile | Pass: Tauri debug application build without bundling |
| Desktop frontend | Pass: Vite production build and 3 Playwright flows |
| JavaScript dependency audit | Pass: 0 known vulnerabilities reported by npm |
| Android | Pass: Rust core for arm64-v8a, armeabi-v7a, and x86_64; Kotlin; unit tests; debug APK |
| iOS Rust core | Pass: cross-target `cargo check` for `aarch64-apple-ios-sim` |

## Native or external gates still required

- Rerun the GitHub Actions matrix from these changes; no remote push or CI run was performed during this workspace pass.
- Build the iOS application and CMIO extension with Xcode on macOS.
- Build and register the DirectShow DLL on Windows.
- Sign, embed, and activate the macOS Camera System Extension with an Apple team and App Group profile.
- Exercise Android and iOS hardware encoders against Linux, macOS, and Windows receivers.
- Confirm 480p/720p/1080p and 15/30/60 FPS in OBS, FaceTime/Photo Booth, and a V4L2 consumer.
- Measure real Wi-Fi and Android USB end-to-end latency; source-level benchmarks are not a substitute for camera-to-consumer timing.
- Create signed release artifacts only after the native matrix and hardware checks pass.

## Release claim boundary

Passing local tests proves the protocol, decoder, conversion, controls, Android build, frontend flows, and Linux compilation. It does not prove operating-system driver installation, camera enumeration, hardware encoding behavior, or production latency. Until the native gates above are recorded, releases should remain pre-release builds.

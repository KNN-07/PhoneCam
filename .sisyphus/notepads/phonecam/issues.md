## 2026-02-10

- `lsp_diagnostics` initially failed because `rust-analyzer` was missing in the toolchain (`Unknown binary 'rust-analyzer'`).
- Resolved by installing the Rust component with `rustup component add rust-analyzer`; diagnostics then ran clean on all changed files.

- Initial connection teardown logic held an extra outbound sender clone in the reader task, preventing outbound channel closure on drop and delaying disconnect observation in tests.
- Resolved by removing reader->writer pong dependency and using one-way ping keepalive semantics; connection drop now closes writer predictably and remote disconnect is observed.

- `ServiceInfo::new(..., &properties)` failed with `IntoTxtProperties` mismatch for `&[(&str, &str); 1]`.
- Resolved by passing a slice (`properties.as_slice()`) so TXT properties satisfy `IntoTxtProperties`.

## 2026-02-10 — TODO 7 (iOS + Rust FFI)

- `xcodebuild` is not available in this Linux environment, so simulator app build verification must be performed on macOS with Xcode installed.
- `lipo` is not available in this Linux environment; the XCFramework script now degrades gracefully to arm64 simulator slice when universal simulator merge is not possible.
- `sourcekit-lsp` is not installed, so Swift LSP diagnostics cannot run in this environment.

## Task 5: Tauri Desktop Skeleton
- **Tauri v2 Build Failure**: Unable to build `phonecam-desktop` backend due to missing system dependencies (`gobject-2.0`, `glib-2.0`, `gio-2.0`, `gtk-3`) in the CI environment.
  - Impact: Cannot run `cargo tauri dev` or `cargo test -p phonecam-desktop`.
  - Workaround: Verified frontend skeleton using `npm run dev` (Vite) and Playwright. The Rust backend code is implemented but not compiled/verified against system libraries.
  - Future Action: Ensure CI environment has `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libssl-dev`, `libwebkit2gtk-4.0-dev`.

## 2026-02-10 (Wave 2)

- Tauri desktop app (TODO 5) requires system GTK dependencies on Linux (`glib-2.0`, `gio-2.0`, `gobject-2.0`, `gdk-3.0`). Build and tests fail on Linux without installing `libgtk-3-dev` and related packages via apt/yum. This is a known Tauri requirement for Linux desktop apps.
- Tauri build verification on Linux requires: `sudo apt-get install libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` (or equivalent for other distros).
- iOS xcodebuild verification requires macOS environment — Linux CI can only verify Rust cross-compilation and tests, not full Xcode builds.

## 2026-02-11 (Wave 2 - Android)

- Android build requires Android SDK configured via ANDROID_HOME or local.properties (sdk.dir). Gradle build fails with "SDK location not found" on CI environments without Android SDK installed.
- cargo-ndk must be installed (`cargo install cargo-ndk`) for cross-compilation to work.
- uniffi-bindgen CLI must be installed (`cargo install uniffi --features cli`) for Kotlin binding generation.

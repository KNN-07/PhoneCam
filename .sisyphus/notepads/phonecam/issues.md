## 2026-02-10

- `lsp_diagnostics` initially failed because `rust-analyzer` was missing in the toolchain (`Unknown binary 'rust-analyzer'`).
- Resolved by installing the Rust component with `rustup component add rust-analyzer`; diagnostics then ran clean on all changed files.

- Initial connection teardown logic held an extra outbound sender clone in the reader task, preventing outbound channel closure on drop and delaying disconnect observation in tests.
- Resolved by removing reader->writer pong dependency and using one-way ping keepalive semantics; connection drop now closes writer predictably and remote disconnect is observed.

- `ServiceInfo::new(..., &properties)` failed with `IntoTxtProperties` mismatch for `&[(&str, &str); 1]`.
- Resolved by passing a slice (`properties.as_slice()`) so TXT properties satisfy `IntoTxtProperties`.

## Task 5: Tauri Desktop Skeleton
- **Tauri v2 Build Failure**: Unable to build `phonecam-desktop` backend due to missing system dependencies (`gobject-2.0`, `glib-2.0`, `gio-2.0`, `gtk-3`) in the CI environment.
  - Impact: Cannot run `cargo tauri dev` or `cargo test -p phonecam-desktop`.
  - Workaround: Verified frontend skeleton using `npm run dev` (Vite) and Playwright. The Rust backend code is implemented but not compiled/verified against system libraries.
  - Future Action: Ensure CI environment has `libgtk-3-dev`, `libayatana-appindicator3-dev`, `librsvg2-dev`, `libssl-dev`, `libwebkit2gtk-4.0-dev`.

## 2026-02-10

- `lsp_diagnostics` initially failed because `rust-analyzer` was missing in the toolchain (`Unknown binary 'rust-analyzer'`).
- Resolved by installing the Rust component with `rustup component add rust-analyzer`; diagnostics then ran clean on all changed files.

- Initial connection teardown logic held an extra outbound sender clone in the reader task, preventing outbound channel closure on drop and delaying disconnect observation in tests.
- Resolved by removing reader->writer pong dependency and using one-way ping keepalive semantics; connection drop now closes writer predictably and remote disconnect is observed.

- `ServiceInfo::new(..., &properties)` failed with `IntoTxtProperties` mismatch for `&[(&str, &str); 1]`.
- Resolved by passing a slice (`properties.as_slice()`) so TXT properties satisfy `IntoTxtProperties`.

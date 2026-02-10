## 2026-02-10

- `lsp_diagnostics` initially failed because `rust-analyzer` was missing in the toolchain (`Unknown binary 'rust-analyzer'`).
- Resolved by installing the Rust component with `rustup component add rust-analyzer`; diagnostics then ran clean on all changed files.

- Initial connection teardown logic held an extra outbound sender clone in the reader task, preventing outbound channel closure on drop and delaying disconnect observation in tests.
- Resolved by removing reader->writer pong dependency and using one-way ping keepalive semantics; connection drop now closes writer predictably and remote disconnect is observed.

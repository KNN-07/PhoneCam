## 2026-02-10

- `lsp_diagnostics` initially failed because `rust-analyzer` was missing in the toolchain (`Unknown binary 'rust-analyzer'`).
- Resolved by installing the Rust component with `rustup component add rust-analyzer`; diagnostics then ran clean on all changed files.

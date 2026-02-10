# PhoneCam

[![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)

PhoneCam is a cross-platform phone-as-webcam application.

## Project Structure

- `rust/`: Core logic and shared crates
  - `phonecam-protocol`: Wire format and protocol definitions
  - `phonecam-transport`: TCP/UDP transport implementation
  - `phonecam-discovery`: mDNS/DNS-SD discovery
  - `phonecam-desktop`: Tauri-based desktop client
  - `phonecam-driver-linux`: V4L2 loopback driver integration for Linux
- `android/`: Native Android application (Kotlin)
- `ios/`: Native iOS application (Swift)
- `apple/`: macOS specific components
- `windows/`: Windows specific driver components

## Development

### Prerequisites

- Rust (latest stable)
- Cargo

### Building

```bash
cargo build --workspace
```

### Testing

```bash
cargo test --workspace
```

## License

This project is licensed under the Apache License, Version 2.0. See [LICENSE](LICENSE) for details.

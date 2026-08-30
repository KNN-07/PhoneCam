#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CRATE_MANIFEST="$ROOT_DIR/rust/phonecam-mobile-core/Cargo.toml"
UDL_FILE="$ROOT_DIR/rust/phonecam-mobile-core/src/phonecam.udl"
GENERATED_DIR="$ROOT_DIR/ios/PhoneCam/PhoneCam/Generated"
TARGET_DIR="$ROOT_DIR/target"
LIB_NAME="libphonecam_mobile_core.a"
OUTPUT_XCFRAMEWORK="$ROOT_DIR/ios/PhoneCam/PhoneCamRust.xcframework"

mkdir -p "$GENERATED_DIR"

echo "[1/5] Ensuring Apple Rust targets are installed"
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

HAVE_X86_SIM=1
if ! rustup target add x86_64-apple-ios >/dev/null 2>&1; then
  HAVE_X86_SIM=0
  echo "warning: unable to install x86_64-apple-ios target, continuing without Intel simulator slice"
fi

echo "[2/5] Building Rust static libraries"
cargo build --locked --manifest-path "$CRATE_MANIFEST" --release --target aarch64-apple-ios
cargo build --locked --manifest-path "$CRATE_MANIFEST" --release --target aarch64-apple-ios-sim

if [[ "$HAVE_X86_SIM" -eq 1 ]]; then
  cargo build --locked --manifest-path "$CRATE_MANIFEST" --release --target x86_64-apple-ios
fi

echo "[3/5] Generating UniFFI Swift bindings"
if ! command -v uniffi-bindgen >/dev/null 2>&1; then
  cargo install uniffi --version 0.31.2 --locked --features cli
fi

uniffi-bindgen generate --language swift "$UDL_FILE" --out-dir "$GENERATED_DIR"

SIM_ARM64_LIB="$TARGET_DIR/aarch64-apple-ios-sim/release/$LIB_NAME"
SIM_LIB="$SIM_ARM64_LIB"

if [[ "$HAVE_X86_SIM" -eq 1 ]]; then
  SIM_X86_LIB="$TARGET_DIR/x86_64-apple-ios/release/$LIB_NAME"
  if [[ -f "$SIM_X86_LIB" ]]; then
    if command -v lipo >/dev/null 2>&1; then
      echo "[4/5] Creating universal simulator static library"
      SIM_UNIVERSAL_DIR="$TARGET_DIR/ios-sim-universal/release"
      mkdir -p "$SIM_UNIVERSAL_DIR"
      lipo -create "$SIM_ARM64_LIB" "$SIM_X86_LIB" -output "$SIM_UNIVERSAL_DIR/$LIB_NAME"
      SIM_LIB="$SIM_UNIVERSAL_DIR/$LIB_NAME"
    else
      echo "warning: lipo not found, using arm64 simulator slice only"
    fi
  fi
fi

if ! command -v xcodebuild >/dev/null 2>&1; then
  echo "[5/5] xcodebuild not found, skipping XCFramework packaging"
  echo "Built static libraries are available under target/<triple>/release"
  exit 0
fi

echo "[5/5] Packaging XCFramework"
rm -rf "$OUTPUT_XCFRAMEWORK"
xcodebuild -create-xcframework \
  -library "$TARGET_DIR/aarch64-apple-ios/release/$LIB_NAME" -headers "$GENERATED_DIR" \
  -library "$SIM_LIB" -headers "$GENERATED_DIR" \
  -output "$OUTPUT_XCFRAMEWORK"

echo "XCFramework ready at $OUTPUT_XCFRAMEWORK"

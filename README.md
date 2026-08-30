# PhoneCam

[![Build and test](https://github.com/KNN-07/PhoneCam/actions/workflows/ci.yml/badge.svg)](https://github.com/KNN-07/PhoneCam/actions/workflows/ci.yml)
[![Developer preview](https://img.shields.io/badge/status-developer%20preview-f59e0b)](docs/release-status.md)
[![License](https://img.shields.io/badge/license-Apache%202.0-2563eb.svg)](LICENSE)

**Your phone is already a great camera. PhoneCam makes it your webcam.**

PhoneCam connects an Android phone or iPhone to Linux, macOS, or Windows and presents its camera as a native desktop video source. Pair over local Wi-Fi in a few taps, or plug in an Android device when you want a stable wired connection. No account, cloud relay, or dedicated webcam required.

> PhoneCam is currently a developer preview distributed from source. The core streaming experience is implemented; signed native-camera packaging and physical-device qualification are still in progress.

## A better camera, already in your pocket

- **Look sharper.** Use the camera hardware and positioning flexibility of a modern phone.
- **Pair without typing addresses.** Find the desktop automatically with local discovery or scan its QR code.
- **Keep video local.** H.264 or HEVC video travels directly between the phone and desktop over your network.
- **Go wired on Android.** An ADB reverse tunnel provides a USB connection when Wi-Fi is crowded.
- **Choose the shot.** Switch cameras and select 480p, 720p, 1080p, 1440p, or 4K at 15, 30, or 60 FPS from the desktop.
- **Choose the codec.** Use H.264, HEVC Main, or Auto. Auto prefers HEVC and falls back to H.264 at the same resolution and frame rate when necessary.
- **Use familiar apps.** PhoneCam targets V4L2 on Linux, Core Media I/O on macOS, and DirectShow on Windows.

## How it works

1. Start PhoneCam on the desktop. It opens a receiver on port `7878` and advertises itself on the local network.
2. Open the mobile app and choose the discovered desktop, or scan the QR code shown by the desktop app.
3. Pick a resolution, frame rate, codec, and camera. PhoneCam exposes only exact profiles supported by both devices and keeps the previous stream active when a change is rejected. Once the native virtual camera is installed, select **PhoneCam** in your video application.

Android users can also connect by USB: authorize the phone with `adb`, choose **Enable Android USB** on the desktop, then tap **Connect via USB** on the phone.

## Platform support

| Component | Connection or output | Preview status |
| --- | --- | --- |
| Android app | Local Wi-Fi, QR pairing, USB via ADB | Builds and unit tests run in CI |
| iOS app | Local Wi-Fi and QR pairing | Source complete; simulator build and unit tests run in CI |
| Linux desktop | V4L2 virtual camera | Locally build-tested; hardware qualification pending |
| macOS desktop | Core Media I/O camera extension | Unsigned build runs in CI; signing and activation pending |
| Windows desktop | DirectShow source filter | Build and format-catalog tests run in CI; registration and hardware qualification pending |

The detailed evidence and release boundaries live in [docs/release-status.md](docs/release-status.md).

## Get started from source

You will need the current stable Rust toolchain and Node.js 22. Desktop builds also need the platform packages required by [Tauri](https://v2.tauri.app/start/prerequisites/).

```bash
git clone https://github.com/KNN-07/PhoneCam.git
cd PhoneCam/rust/phonecam-desktop
npm ci
npm run tauri dev
```

The desktop starts in listening mode. Open the mobile app on the same network, then use discovery or the QR fallback to connect.

### Linux virtual camera

Install `v4l2loopback`, load a PhoneCam device, and point the desktop at it:

```bash
sudo modprobe v4l2loopback devices=1 video_nr=10 card_label=PhoneCam exclusive_caps=1
PHONECAM_V4L2_DEVICE=/dev/video10 npm run tauri dev
```

### Android app

Android builds require JDK 17, Android SDK 34, build tools 34.0.0, and NDK 26.1.10909125.

```bash
cd android
./gradlew --no-daemon testDebugUnitTest assembleDebug
```

### iOS app

iOS builds require a current Xcode installation. Build the Rust XCFramework and generated Swift bindings before opening or building the project:

```bash
bash ios/build-xcframework.sh
xcodebuild \
  -project ios/PhoneCam/PhoneCam.xcodeproj \
  -scheme PhoneCam \
  -configuration Release \
  -sdk iphonesimulator \
  -destination 'generic/platform=iOS Simulator' \
  CODE_SIGNING_ALLOWED=NO \
  build
```

Run the pure Swift profile, bitrate, format-ranking, and AVCC/HVCC packet tests in an iOS simulator:

```bash
xcodebuild \
  -project ios/PhoneCam/PhoneCam.xcodeproj \
  -scheme PhoneCam \
  -destination 'platform=iOS Simulator,name=iPhone 16,OS=latest' \
  CODE_SIGNING_ALLOWED=NO \
  test
```

### Native desktop camera outputs

- **macOS:** build and sign `apple/PhoneCamDriver` as a Camera System Extension using the `group.com.phonecam.shared` App Group. macOS requires installation in `/Applications` and explicit user approval.
- **Windows:** build `windows/PhoneCamDriver` with Visual Studio 2022, Ninja, and CMake, run `ctest --test-dir build/windows-driver --output-on-failure`, place the DLL in a stable location, then register it from an elevated terminal with `regsvr32`.

Native camera activation changes the host operating system, so the source build deliberately does not perform it automatically.

## Build confidence on every change

The [build-and-test workflow](.github/workflows/ci.yml) runs for pull requests, pushes to `main`, and manual dispatches. It checks:

- formatting, Clippy, and Rust tests across Linux, macOS, and Windows;
- the Vite build, dependency audit, Playwright product flows, and integrated Tauri build;
- Android Rust/JNI bindings, Kotlin unit tests, and a debug APK;
- the iOS Rust XCFramework, Swift simulator tests, simulator app, macOS camera extension, Windows camera DLL, and Windows format-catalog CTest;
- Rust coverage, published as a workflow artifact.

Tags matching `v*` invoke the same complete quality gate before the [release workflow](.github/workflows/release.yml) creates Linux, macOS, and Windows desktop bundles. Releases remain drafts and pre-releases while native signing and real-device qualification are outstanding.

Run the core checks locally from the repository root:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked

cd rust/phonecam-desktop
npm ci
npm audit --audit-level=high
npm run build
npx playwright install chromium
npm test
npm run tauri -- build --debug --no-bundle
```

## Built as one pipeline

PhoneCam shares its protocol, transport, discovery, and mobile FFI code in Rust. Native Android and iOS capture layers advertise exact camera/encoder profiles and encode H.264 or HEVC Main. The Tauri desktop receiver negotiates the codec, resolution, and frame rate, decodes into NV12, and commits the matching native output format before handing frames to the operating system’s camera interface.

```text
Phone camera -> H.264/HEVC stream -> local transport -> desktop decoder -> virtual camera -> your app
```

The codebase is organized around that path:

- `rust/phonecam-protocol` — wire messages and framing
- `rust/phonecam-transport` — TCP lifecycle and keepalive
- `rust/phonecam-discovery` — mDNS/DNS-SD and QR connection URIs
- `rust/phonecam-mobile-core` — shared Android/iOS FFI core
- `rust/phonecam-desktop` — Tauri receiver, controls, and decode pipeline
- `android` and `ios` — native mobile camera applications
- `rust/phonecam-driver-linux`, `apple/PhoneCamDriver`, and `windows/PhoneCamDriver` — native virtual-camera outputs

## License

PhoneCam is licensed under the [Apache License 2.0](LICENSE).

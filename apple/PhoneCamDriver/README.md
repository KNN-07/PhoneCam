# PhoneCam macOS Camera Extension

A CoreMediaIO (CMIO) Camera Extension for macOS 12.3+ that provides a virtual webcam device called "PhoneCam".

## Overview

This Camera Extension receives video frames via IPC from the PhoneCam desktop Tauri app and presents them as a system camera that can be used by any macOS application (FaceTime, Zoom, OBS, etc.).

## Architecture

```
Tauri Desktop App → IPC (UNIX Socket) → Camera Extension → CMIO → System
                      (App Group Container)
```

### Components

- **CameraExtensionProvider.swift**: Main provider implementing `CMIOExtensionProviderSource`
- **PhoneCamDeviceSource**: Device source managing the virtual camera device
- **PhoneCamStreamSource**: Stream source handling video format and streaming state
- **IPCReceiver**: UNIX socket server receiving NV12 frames from Tauri
- **FrameBufferQueue**: Thread-safe ring buffer for smooth frame delivery

## IPC Protocol

Frames are sent from the Tauri app via UNIX socket in the App Group container:

```
Socket Path: ~/Library/Group Containers/group.com.phonecam.shared/phonecam.sock

Frame Format:
[4 bytes]  width (uint32, little-endian)
[4 bytes]  height (uint32, little-endian)
[8 bytes]  timestamp (uint64 nanoseconds, little-endian)
[N bytes]  NV12 pixel data (width * height * 1.5 bytes)
```

## Requirements

- macOS 12.3 or later (CMIO Camera Extensions require 12.3+)
- Xcode 14.0+
- Valid Apple Developer account for code signing
- App Group entitlement configured

## Building

### Prerequisites

1. Configure App Group ID in `PhoneCamDriver.entitlements`:
   ```xml
   <key>com.apple.security.application-groups</key>
   <array>
       <string>group.com.phonecam.shared</string>
   </array>
   ```

2. Update bundle identifiers in project settings if needed

3. Configure code signing with your Apple Developer account

### Build Commands

```bash
# Open in Xcode
open PhoneCamDriver.xcodeproj

# Or build from command line
xcodebuild -project PhoneCamDriver.xcodeproj \
    -scheme PhoneCamDriver \
    -configuration Release \
    -derivedDataPath build \
    build
```

## Installation

The Xcode target builds `PhoneCamDriver.systemextension`. A signed release must embed it at
`PhoneCam.app/Contents/Library/SystemExtensions/PhoneCamDriver.systemextension`, then the
containing app must submit an `OSSystemExtensionRequest.activationRequest` for
`com.phonecam.driver.cameraextension`. The user approves the request in System Settings →
Privacy & Security.

Embedding, signing, and activation are native release gates and are not automated by the
current Tauri bundle. `systemextensionsctl list` can be used to inspect activation state; it is
not an installer for a standalone camera extension.

## Configuration

The extension reads configuration from `Info.plist`:

- `PhoneCamAppGroupIdentifier`: App Group ID for IPC
- `PhoneCamDeviceUUID`: UUID for the virtual camera device
- `PhoneCamStreamUUID`: UUID for the video stream

## Testing

1. Build and run the PhoneCam desktop app
2. Enable the Camera Extension when prompted
3. Open FaceTime or Photo Booth
4. Select "PhoneCam" from the camera list
5. Connect a phone and start streaming

## Troubleshooting

### Extension Not Appearing

- Check code signing is valid
- Verify App Group ID matches between app and extension
- Check Console.app for extension logs
- Run `systemextensionsctl list` to see extension state

### No Video

- Verify IPC socket path is accessible
- Check that Tauri app has created the socket
- Review extension logs in Console.app

### Build Errors

- Ensure macOS deployment target is 12.3 or later
- Verify all CMIO framework imports are available
- Check that entitlements file is properly configured

## References

- [Apple CMIO Camera Extension Documentation](https://developer.apple.com/documentation/coremediaio/creating-a-camera_extension_with_core_media_i_o)
- [WWDC22: Create Camera Extensions](https://developer.apple.com/videos/play/wwdc2022/10022/)
- [OBS macOS Virtual Camera](https://github.com/obsproject/obs-studio/tree/master/plugins/mac-virtualcam/src/camera-extension)
- [Halle/SinkCam Example](https://github.com/Halle/SinkCam)

## License

Apache License 2.0 - See LICENSE file in project root

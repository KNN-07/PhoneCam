# PhoneCam Windows Virtual Camera Driver

A DirectShow source filter that provides a virtual webcam device called "PhoneCam" for Windows.

## Overview

This DirectShow filter receives video frames via named pipe from the PhoneCam desktop Tauri app and presents them as a system camera that can be used by any Windows application (Zoom, Teams, OBS, Chrome, etc.).

## Architecture

```
Tauri Desktop App → Named Pipe → DirectShow Filter → DirectShow Graph → Application
                     (\\.\pipe\PhoneCam)
```

### Components

- **dllmain.cpp**: DLL entry point and COM registration
- **filter.cpp/.h**: Main DirectShow filter implementing `IBaseFilter`, `IMediaFilter`, `IPersist`
- **output_pin.cpp/.h**: Output pin delivering frames, implementing `IPin`, `IAMStreamConfig`, `IKsPropertySet`
- **frame_receiver.cpp/.h**: Named pipe server receiving NV12 frames from Tauri
- **guids.cpp/.h**: GUID definitions for COM classes

## Named Pipe Protocol

Frames are sent from the Tauri app via Windows named pipe:

```
Pipe Name: \\.\pipe\PhoneCam

Frame Format:
[4 bytes]  width (uint32, little-endian)
[4 bytes]  height (uint32, little-endian)
[8 bytes]  timestamp (uint64 nanoseconds, little-endian)
[N bytes]  NV12 pixel data (width * height * 1.5 bytes)
```

## Requirements

- Windows 10 or later
- Visual Studio 2019 or later (or MinGW-w64)
- CMake 3.16+
- Windows SDK

## Building

### Prerequisites

Install Visual Studio 2019 or later with:
- Desktop development with C++ workload
- Windows SDK
- CMake tools

### Build Commands

```powershell
# Create build directory
mkdir build
cd build

# Configure with CMake
cmake .. -A x64

# Build
cmake --build . --config Release
```

### MinGW Build (Alternative)

```bash
# Install MinGW-w64 and CMake
mkdir build
cd build
cmake .. -G "MinGW Makefiles" -DCMAKE_BUILD_TYPE=Release
mingw32-make
```

## Installation

### Register the Filter

```powershell
# As Administrator
regsvr32 PhoneCamDriver.dll
```

### Unregister

```powershell
# As Administrator
regsvr32 /u PhoneCamDriver.dll
```

### Manual Registration

If you prefer not to use regsvr32, you can manually register:

1. Copy `PhoneCamDriver.dll` to a permanent location (e.g., `C:\Program Files\PhoneCam\`)
2. Add the path to the DLL to the `Path` environment variable
3. Import registry entries (see below)

### Registry Entries

The filter registers under:
- `HKEY_CLASSES_ROOT\CLSID\{B9F8C77E-3B6A-4B3B-9E8A-9C5F7B1D2E3A}` - Filter class
- `HKEY_CLASSES_ROOT\CLSID\{B9F8C77E-3B6A-4B3B-9E8A-9C5F7B1D2E3A}\InprocServer32` - DLL path
- `HKEY_CLASSES_ROOT\CLSID\{860BB310-5D01-11d0-BD3B-00A0C911CE86}\Instance\{B9F8C77E-3B6A-4B3B-9E8A-9C5F7B1D2E3A}` - Video Input Device category

## Testing

1. Build and register the DLL
2. Start the PhoneCam desktop app
3. Open any camera app (Camera app, Zoom, Teams, OBS)
4. Select "PhoneCam" from the camera list
5. Connect a phone and start streaming

## Troubleshooting

### Filter Not Appearing

- Ensure DLL is registered (run `regsvr32` as Administrator)
- Check registry entries exist under `CLSID`
- Verify DLL dependencies are satisfied (use Dependency Walker)

### No Video

- Check that named pipe is created by Tauri app (`\\.\pipe\PhoneCam`)
- Verify Tauri app has proper permissions
- Check Event Viewer for errors

### Build Errors

- Ensure Windows SDK is installed
- For Visual Studio: Use x64 Native Tools Command Prompt
- For MinGW: Ensure mingw32-make is in PATH

## Media Types Supported

The filter supports the following formats:

- **NV12**: Primary format (420 planar, 8-bit)
- **YUY2/YUYV**: Secondary format (packed YUV)
- **Resolutions**: 640x480 (480p), 1280x720 (720p), 1920x1080 (1080p)
- **Frame Rate**: 30 FPS (configurable)

## Security Notes

- The filter runs in-process with the consuming application
- Named pipe uses default Windows security (accessible to same user)
- No elevation required for filter operation (only for registration)

## References

- [DirectShow Documentation](https://docs.microsoft.com/en-us/windows/win32/directshow/directshow)
- [OBS Virtual Camera](https://github.com/Fenrirthviti/obs-virtual-cam)
- [DirectShow Base Classes](https://github.com/roman380/tmhare.mvps.org-vcam)
- [Windows Named Pipes](https://docs.microsoft.com/en-us/windows/win32/ipc/named-pipes)

## License

Apache License 2.0 - See LICENSE file in project root

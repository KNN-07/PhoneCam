import SwiftUI

@main
struct PhoneCamApp: App {
    @StateObject private var cameraController: CameraController
    @StateObject private var streamManager: StreamManager

    init() {
        let cameraController = CameraController()
        let streamManager = StreamManager(cameraController: cameraController)

        _cameraController = StateObject(wrappedValue: cameraController)
        _streamManager = StateObject(wrappedValue: streamManager)

        let uniffiMessage = ffiTestMessage()
        print("FFI test (UniFFI): \(uniffiMessage)")

        if let ptr = phonecam_ffi_test_message() {
            let message = String(cString: ptr)
            print("FFI test (raw C): \(message)")
            phonecam_string_free(ptr)
        } else {
            print("FFI test (raw C): failed to fetch Rust message")
        }
    }

    var body: some Scene {
        WindowGroup {
            ContentView(
                cameraController: cameraController,
                streamManager: streamManager
            )
        }
    }
}

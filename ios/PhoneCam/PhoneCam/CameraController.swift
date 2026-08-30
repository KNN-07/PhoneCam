import AVFoundation
import Combine
import Foundation

enum CaptureResolution: String, CaseIterable, Identifiable {
    case p480 = "480p"
    case p720 = "720p"
    case p1080 = "1080p"

    var id: String { rawValue }

    var dimensions: (width: Int32, height: Int32) {
        switch self {
        case .p480:
            return (640, 480)
        case .p720:
            return (1280, 720)
        case .p1080:
            return (1920, 1080)
        }
    }

    var dimensionsU16: (width: UInt16, height: UInt16) {
        let size = dimensions
        return (UInt16(size.width), UInt16(size.height))
    }

    var sessionPreset: AVCaptureSession.Preset {
        switch self {
        case .p480:
            return .vga640x480
        case .p720:
            return .hd1280x720
        case .p1080:
            return .hd1920x1080
        }
    }

    var targetBitrate: Int {
        switch self {
        case .p480:
            return 2_000_000
        case .p720:
            return 4_000_000
        case .p1080:
            return 5_000_000
        }
    }

    static func matching(width: UInt16, height: UInt16) -> CaptureResolution? {
        allCases.first { resolution in
            let dimensions = resolution.dimensionsU16
            return dimensions.width == width && dimensions.height == height
        }
    }
}

final class CameraController: NSObject, ObservableObject {
    let session = AVCaptureSession()

    @Published private(set) var authorizationStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @Published private(set) var isSessionRunning = false
    @Published private(set) var interruptionDescription: String?
    @Published private(set) var activeCameraPosition: AVCaptureDevice.Position = .back

    var isUsingFrontCamera: Bool {
        activeCameraPosition == .front
    }

    var onSampleBuffer: ((CMSampleBuffer) -> Void)?

    private let sessionQueue = DispatchQueue(label: "com.phonecam.ios.camera.session")
    private let outputQueue = DispatchQueue(label: "com.phonecam.ios.camera.output")
    private let videoOutput = AVCaptureVideoDataOutput()

    private var isConfigured = false
    private var configuredResolution: CaptureResolution = .p720
    private var configuredFps: Int32 = 30
    private var notificationTokens: [NSObjectProtocol] = []

    deinit {
        notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    }

    func requestAccessAndConfigure(
        resolution: CaptureResolution,
        fps: Int32,
        completion: @escaping (Bool) -> Void
    ) {
        let status = AVCaptureDevice.authorizationStatus(for: .video)
        DispatchQueue.main.async {
            self.authorizationStatus = status
        }

        switch status {
        case .authorized:
            configureSession(for: resolution, fps: fps, completion: completion)
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                guard let self else {
                    DispatchQueue.main.async { completion(false) }
                    return
                }

                let updatedStatus = AVCaptureDevice.authorizationStatus(for: .video)
                DispatchQueue.main.async {
                    self.authorizationStatus = updatedStatus
                }

                guard granted else {
                    DispatchQueue.main.async { completion(false) }
                    return
                }

                self.configureSession(for: resolution, fps: fps, completion: completion)
            }
        case .denied, .restricted:
            DispatchQueue.main.async {
                completion(false)
            }
        @unknown default:
            DispatchQueue.main.async {
                completion(false)
            }
        }
    }

    func startSession() {
        sessionQueue.async {
            guard self.authorizationStatus == .authorized, self.isConfigured else {
                return
            }

            if !self.session.isRunning {
                self.session.startRunning()
            }

            DispatchQueue.main.async {
                self.isSessionRunning = self.session.isRunning
            }
        }
    }

    func stopSession() {
        sessionQueue.async {
            guard self.session.isRunning else {
                return
            }

            self.session.stopRunning()
            DispatchQueue.main.async {
                self.isSessionRunning = false
            }
        }
    }

    func switchCamera(toFront: Bool, completion: @escaping (Bool, Bool) -> Void) {
        sessionQueue.async {
            guard self.isConfigured else {
                DispatchQueue.main.async {
                    completion(false, self.isUsingFrontCamera)
                }
                return
            }

            let targetPosition: AVCaptureDevice.Position = toFront ? .front : .back
            let fallbackPosition: AVCaptureDevice.Position = toFront ? .back : .front

            guard
                let currentInput = self.session.inputs.first(where: { $0 is AVCaptureDeviceInput }) as? AVCaptureDeviceInput,
                let replacementDevice =
                    Self.cameraDevice(for: targetPosition)
                    ?? Self.cameraDevice(for: fallbackPosition)
                    ?? AVCaptureDevice.default(for: .video)
            else {
                DispatchQueue.main.async {
                    completion(false, self.isUsingFrontCamera)
                }
                return
            }

            let wasRunning = self.session.isRunning
            if wasRunning {
                self.session.stopRunning()
            }

            self.session.beginConfiguration()
            self.session.removeInput(currentInput)

            var switched = false
            do {
                let replacementInput = try AVCaptureDeviceInput(device: replacementDevice)
                if self.session.canAddInput(replacementInput) {
                    self.session.addInput(replacementInput)

                    if Self.applyCaptureFormat(
                        resolution: self.configuredResolution,
                        fps: self.configuredFps,
                        to: replacementDevice
                    ) {
                        self.activeCameraPosition = replacementDevice.position
                        self.configureVideoOutputConnection()
                        switched = true
                    } else {
                        self.session.removeInput(replacementInput)
                    }
                }
            } catch {
                switched = false
            }

            if !switched, self.session.canAddInput(currentInput) {
                self.session.addInput(currentInput)
                self.activeCameraPosition = currentInput.device.position
                self.configureVideoOutputConnection()
            }

            self.session.commitConfiguration()

            if wasRunning {
                self.session.startRunning()
            }

            DispatchQueue.main.async {
                self.isSessionRunning = self.session.isRunning
                completion(switched, self.isUsingFrontCamera)
            }
        }
    }

    private func configureSession(
        for resolution: CaptureResolution,
        fps: Int32,
        completion: @escaping (Bool) -> Void
    ) {
        sessionQueue.async {
            let wasRunning = self.session.isRunning

            if
                self.isConfigured,
                self.configuredResolution == resolution,
                self.configuredFps == fps
            {
                DispatchQueue.main.async {
                    completion(true)
                }
                return
            }

            self.session.beginConfiguration()
            defer {
                self.session.commitConfiguration()
            }
            self.isConfigured = false

            if self.session.canSetSessionPreset(resolution.sessionPreset) {
                self.session.sessionPreset = resolution.sessionPreset
            } else {
                self.session.sessionPreset = .high
            }

            self.session.inputs.forEach { self.session.removeInput($0) }
            self.session.outputs.forEach { self.session.removeOutput($0) }

            guard
                let cameraDevice =
                    Self.cameraDevice(for: .back)
                    ?? Self.cameraDevice(for: .front)
                    ?? AVCaptureDevice.default(for: .video)
            else {
                DispatchQueue.main.async {
                    completion(false)
                }
                return
            }

            do {
                let cameraInput = try AVCaptureDeviceInput(device: cameraDevice)
                guard self.session.canAddInput(cameraInput) else {
                    DispatchQueue.main.async {
                        completion(false)
                    }
                    return
                }

                self.session.addInput(cameraInput)
                self.activeCameraPosition = cameraDevice.position

                if self.session.canSetSessionPreset(.inputPriority) {
                    self.session.sessionPreset = .inputPriority
                }

                guard Self.applyCaptureFormat(resolution: resolution, fps: fps, to: cameraDevice) else {
                    DispatchQueue.main.async {
                        completion(false)
                    }
                    return
                }
            } catch {
                DispatchQueue.main.async {
                    completion(false)
                }
                return
            }

            self.videoOutput.alwaysDiscardsLateVideoFrames = true
            self.videoOutput.videoSettings = [
                kCVPixelBufferPixelFormatTypeKey as String: Int(kCVPixelFormatType_420YpCbCr8BiPlanarFullRange),
            ]
            self.videoOutput.setSampleBufferDelegate(self, queue: self.outputQueue)

            guard self.session.canAddOutput(self.videoOutput) else {
                DispatchQueue.main.async {
                    completion(false)
                }
                return
            }

            self.session.addOutput(self.videoOutput)
            self.configureVideoOutputConnection()

            self.isConfigured = true
            self.configuredResolution = resolution
            self.configuredFps = fps
            self.registerSessionObserversIfNeeded()

            if wasRunning, !self.session.isRunning {
                self.session.startRunning()
            }

            DispatchQueue.main.async {
                self.isSessionRunning = self.session.isRunning
                completion(true)
            }
        }
    }

    private func configureVideoOutputConnection() {
        guard let connection = videoOutput.connection(with: .video) else {
            return
        }

        if connection.isVideoOrientationSupported {
            connection.videoOrientation = .portrait
        }

        if connection.isVideoMirroringSupported {
            connection.isVideoMirrored = false
        }
    }

    private static func cameraDevice(for position: AVCaptureDevice.Position) -> AVCaptureDevice? {
        AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: position)
            ?? AVCaptureDevice.DiscoverySession(
                deviceTypes: [.builtInWideAngleCamera],
                mediaType: .video,
                position: position
            ).devices.first
    }

    private static func applyCaptureFormat(
        resolution: CaptureResolution,
        fps: Int32,
        to device: AVCaptureDevice
    ) -> Bool {
        let requestedFps = Double(fps)
        let dimensions = resolution.dimensions
        guard
            fps > 0,
            let format = device.formats
                .filter({ format in
                    let formatDimensions = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
                    return
                        formatDimensions.width == dimensions.width
                        && formatDimensions.height == dimensions.height
                        && format.videoSupportedFrameRateRanges.contains(where: { range in
                            range.minFrameRate <= requestedFps && requestedFps <= range.maxFrameRate
                        })
                })
                .max(by: { lhs, rhs in
                    let lhsMaximum = lhs.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
                    let rhsMaximum = rhs.videoSupportedFrameRateRanges.map(\.maxFrameRate).max() ?? 0
                    return lhsMaximum < rhsMaximum
                })
        else {
            return false
        }

        do {
            try device.lockForConfiguration()
            defer { device.unlockForConfiguration() }

            device.activeFormat = format
            let frameDuration = CMTime(value: 1, timescale: fps)
            device.activeVideoMinFrameDuration = frameDuration
            device.activeVideoMaxFrameDuration = frameDuration
            return true
        } catch {
            return false
        }
    }

    private func registerSessionObserversIfNeeded() {
        guard notificationTokens.isEmpty else {
            return
        }

        let center = NotificationCenter.default

        notificationTokens.append(
            center.addObserver(
                forName: .AVCaptureSessionWasInterrupted,
                object: session,
                queue: .main
            ) { [weak self] notification in
                self?.interruptionDescription = Self.interruptionMessage(from: notification)
            }
        )

        notificationTokens.append(
            center.addObserver(
                forName: .AVCaptureSessionInterruptionEnded,
                object: session,
                queue: .main
            ) { [weak self] _ in
                self?.interruptionDescription = nil
            }
        )

        notificationTokens.append(
            center.addObserver(
                forName: .AVCaptureSessionRuntimeError,
                object: session,
                queue: .main
            ) { [weak self] notification in
                guard let self else {
                    return
                }

                if
                    let error = notification.userInfo?[AVCaptureSessionErrorKey] as? AVError,
                    error.code == .mediaServicesWereReset
                {
                    self.startSession()
                    return
                }

                self.interruptionDescription = "Camera runtime error"
            }
        )
    }

    private static func interruptionMessage(from notification: Notification) -> String {
        guard
            let reasonValue = notification.userInfo?[AVCaptureSessionInterruptionReasonKey] as? NSNumber,
            let reason = AVCaptureSession.InterruptionReason(rawValue: reasonValue.intValue)
        else {
            return "Camera session interrupted"
        }

        switch reason {
        case .audioDeviceInUseByAnotherClient:
            return "Camera audio device is in use by another app"
        case .videoDeviceInUseByAnotherClient:
            return "Camera is in use by another app"
        case .videoDeviceNotAvailableInBackground:
            return "Camera is unavailable in background"
        case .videoDeviceNotAvailableWithMultipleForegroundApps:
            return "Camera unavailable with multiple foreground apps"
        case .videoDeviceNotAvailableDueToSystemPressure:
            return "Camera paused due to system pressure"
        @unknown default:
            return "Camera session interrupted"
        }
    }
}

extension CameraController: AVCaptureVideoDataOutputSampleBufferDelegate {
    func captureOutput(
        _ output: AVCaptureOutput,
        didOutput sampleBuffer: CMSampleBuffer,
        from connection: AVCaptureConnection
    ) {
        onSampleBuffer?(sampleBuffer)
    }
}

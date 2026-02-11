import AVFoundation
import Combine
import Foundation

enum CaptureResolution: String, CaseIterable, Identifiable {
    case p720 = "720p"
    case p1080 = "1080p"

    var id: String { rawValue }

    var dimensions: (width: Int32, height: Int32) {
        switch self {
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
        case .p720:
            return .hd1280x720
        case .p1080:
            return .hd1920x1080
        }
    }

    var targetBitrate: Int {
        switch self {
        case .p720:
            return 4_000_000
        case .p1080:
            return 5_000_000
        }
    }
}

final class CameraController: NSObject, ObservableObject {
    let session = AVCaptureSession()

    @Published private(set) var authorizationStatus: AVAuthorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
    @Published private(set) var isSessionRunning = false
    @Published private(set) var interruptionDescription: String?

    var onSampleBuffer: ((CMSampleBuffer) -> Void)?

    private let sessionQueue = DispatchQueue(label: "com.phonecam.ios.camera.session")
    private let outputQueue = DispatchQueue(label: "com.phonecam.ios.camera.output")
    private let videoOutput = AVCaptureVideoDataOutput()

    private var isConfigured = false
    private var configuredResolution: CaptureResolution = .p720
    private var notificationTokens: [NSObjectProtocol] = []

    deinit {
        notificationTokens.forEach { NotificationCenter.default.removeObserver($0) }
    }

    func requestAccessAndConfigure(
        resolution: CaptureResolution,
        completion: @escaping (Bool) -> Void
    ) {
        let status = AVCaptureDevice.authorizationStatus(for: .video)
        DispatchQueue.main.async {
            self.authorizationStatus = status
        }

        switch status {
        case .authorized:
            configureSession(for: resolution, completion: completion)
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

                self.configureSession(for: resolution, completion: completion)
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

    private func configureSession(
        for resolution: CaptureResolution,
        completion: @escaping (Bool) -> Void
    ) {
        sessionQueue.async {
            let wasRunning = self.session.isRunning

            if self.isConfigured, self.configuredResolution == resolution {
                DispatchQueue.main.async {
                    completion(true)
                }
                return
            }

            self.session.beginConfiguration()
            defer {
                self.session.commitConfiguration()
            }

            if self.session.canSetSessionPreset(resolution.sessionPreset) {
                self.session.sessionPreset = resolution.sessionPreset
            } else {
                self.session.sessionPreset = .high
            }

            self.session.inputs.forEach { self.session.removeInput($0) }
            self.session.outputs.forEach { self.session.removeOutput($0) }

            guard
                let cameraDevice = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
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

            if let connection = self.videoOutput.connection(with: .video) {
                if connection.isVideoOrientationSupported {
                    connection.videoOrientation = .portrait
                }
                if connection.isVideoMirroringSupported {
                    connection.isVideoMirrored = false
                }
            }

            self.isConfigured = true
            self.configuredResolution = resolution
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

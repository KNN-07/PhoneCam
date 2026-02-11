import Foundation

final class StreamManager: ObservableObject {
    @Published private(set) var statusText = "Idle"
    @Published private(set) var isConnected = false
    @Published var selectedResolution: CaptureResolution = .p720

    private let cameraController: CameraController
    private var encoder: H264Encoder?
    private var connectionStatusTimer: Timer?
    private var isStreaming = false

    private var endpointHost: String = "127.0.0.1"
    private var endpointPort: UInt16 = 7878

    private struct QrConnectionInfo {
        let host: String
        let port: UInt16
        let name: String
    }

    init(cameraController: CameraController) {
        self.cameraController = cameraController
    }

    deinit {
        connectionStatusTimer?.invalidate()
    }

    func startStreaming(host: String = "127.0.0.1", port: UInt16 = 7878) {
        guard !isStreaming else {
            return
        }

        endpointHost = host
        endpointPort = port

        DispatchQueue.main.async {
            self.statusText = "Requesting camera permission…"
        }

        cameraController.requestAccessAndConfigure(resolution: selectedResolution) { [weak self] granted in
            guard let self else {
                return
            }

            guard granted else {
                DispatchQueue.main.async {
                    self.statusText = "Camera permission denied"
                    self.isConnected = false
                }
                return
            }

            self.cameraController.onSampleBuffer = { [weak self] sampleBuffer in
                self?.encoder?.encode(sampleBuffer: sampleBuffer)
            }

            self.configureEncoder(for: self.selectedResolution)
            self.applyVideoDimensionsToRust()
            self.initializeTransport()

            self.cameraController.startSession()
            self.startConnectionStatusPolling()

            DispatchQueue.main.async {
                self.isStreaming = true
                self.statusText = self.isConnected
                    ? "Streaming (\(self.selectedResolution.rawValue))"
                    : "Camera active, transport disconnected"
            }
        }
    }

    func stopStreaming() {
        guard isStreaming else {
            return
        }

        cameraController.onSampleBuffer = nil
        cameraController.stopSession()

        encoder?.stop()
        encoder = nil

        phonecam_transport_shutdown()

        connectionStatusTimer?.invalidate()
        connectionStatusTimer = nil

        DispatchQueue.main.async {
            self.isStreaming = false
            self.isConnected = false
            self.statusText = "Stopped"
        }
    }

    func setResolution(_ resolution: CaptureResolution) {
        guard selectedResolution != resolution else {
            return
        }

        selectedResolution = resolution

        guard isStreaming else {
            return
        }

        cameraController.stopSession()

        cameraController.requestAccessAndConfigure(resolution: resolution) { [weak self] granted in
            guard let self else {
                return
            }

            guard granted else {
                DispatchQueue.main.async {
                    self.statusText = "Camera permission denied"
                }
                return
            }

            self.configureEncoder(for: resolution)
            self.applyVideoDimensionsToRust()
            self.cameraController.startSession()

            DispatchQueue.main.async {
                self.statusText = self.isConnected
                    ? "Streaming (\(resolution.rawValue))"
                    : "Camera active, transport disconnected"
            }
        }
    }

    @discardableResult
    func connectUsingQrUri(_ uri: String) -> Bool {
        guard let qrInfo = parseQrCodeUri(uri) else {
            DispatchQueue.main.async {
                self.statusText = "Invalid PhoneCam QR code"
            }
            return false
        }

        endpointHost = qrInfo.host
        endpointPort = qrInfo.port

        if isStreaming {
            phonecam_transport_shutdown()
            let connected = initializeTransport()
            DispatchQueue.main.async {
                self.statusText = connected
                    ? "Connected to \(qrInfo.name) (\(qrInfo.host):\(qrInfo.port))"
                    : "Unable to connect to \(qrInfo.name) (\(qrInfo.host):\(qrInfo.port))"
            }
            return connected
        }

        startStreaming(host: qrInfo.host, port: qrInfo.port)
        return true
    }

    private func configureEncoder(for resolution: CaptureResolution) {
        let size = resolution.dimensions

        do {
            if let encoder {
                try encoder.updateConfiguration(
                    width: size.width,
                    height: size.height,
                    fps: 30,
                    bitrate: resolution.targetBitrate
                )
            } else {
                let newEncoder = H264Encoder(
                    width: size.width,
                    height: size.height,
                    fps: 30,
                    bitrate: resolution.targetBitrate
                )

                newEncoder.onEncodedNALUnit = { nalUnit, ptsUs, isKeyframe in
                    nalUnit.withUnsafeBytes { rawBuffer in
                        guard
                            let baseAddress = rawBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self)
                        else {
                            return
                        }

                        phonecam_send_video_frame(baseAddress, rawBuffer.count, ptsUs, isKeyframe)
                    }
                }

                try newEncoder.start()
                encoder = newEncoder
            }
        } catch {
            DispatchQueue.main.async {
                self.statusText = "Encoder error: \(error.localizedDescription)"
            }
        }
    }

    private func applyVideoDimensionsToRust() {
        let dimensions = selectedResolution.dimensionsU16
        phonecam_set_video_dimensions(dimensions.width, dimensions.height)
    }

    @discardableResult
    private func initializeTransport() -> Bool {
        let initialized = endpointHost.withCString { hostCString in
            phonecam_transport_init(hostCString, endpointPort)
        }

        let connected = initialized && phonecam_transport_is_connected()

        DispatchQueue.main.async {
            self.isConnected = connected
            if connected {
                self.statusText = "Connected to \(self.endpointHost):\(self.endpointPort)"
            } else {
                self.statusText = "Unable to connect to \(self.endpointHost):\(self.endpointPort)"
            }
        }

        return connected
    }

    private func parseQrCodeUri(_ uri: String) -> QrConnectionInfo? {
        let trimmed = uri.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else {
            return nil
        }

        let parsedPtr: UnsafeMutablePointer<CChar>? = trimmed.withCString { rawUri in
            phonecam_parse_qr_code_uri(rawUri)
        }

        guard let parsedPtr else {
            return nil
        }

        defer {
            phonecam_string_free(parsedPtr)
        }

        let payload = String(cString: parsedPtr)
        let parts = payload.split(separator: "|", maxSplits: 2, omittingEmptySubsequences: false)
        guard parts.count == 3 else {
            return nil
        }

        let host = String(parts[0]).trimmingCharacters(in: .whitespacesAndNewlines)
        let port = UInt16(String(parts[1]))
        let name = String(parts[2]).trimmingCharacters(in: .whitespacesAndNewlines)
        guard !host.isEmpty, let port else {
            return nil
        }

        return QrConnectionInfo(
            host: host,
            port: port,
            name: name.isEmpty ? "PhoneCam Desktop" : name
        )
    }

    private func startConnectionStatusPolling() {
        connectionStatusTimer?.invalidate()

        let timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            guard let self else {
                return
            }

            let connected = phonecam_transport_is_connected()
            guard connected != self.isConnected else {
                return
            }

            self.isConnected = connected

            if connected {
                self.statusText = "Streaming (\(self.selectedResolution.rawValue))"
            } else if self.isStreaming {
                self.statusText = "Camera active, transport disconnected"
            }
        }

        RunLoop.main.add(timer, forMode: .common)
        connectionStatusTimer = timer
    }
}

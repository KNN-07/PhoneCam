import AVFoundation
import VideoToolbox
import Foundation
enum VideoCodec: String, Codable, CaseIterable {
    case h264
    case hevc

    var nativeID: UInt8 { self == .h264 ? 0 : 1 }
    var cmCodecType: CMVideoCodecType {
        self == .h264 ? kCMVideoCodecType_H264 : kCMVideoCodecType_HEVC
    }
    var profileLevel: CFString {
        self == .h264 ? kVTProfileLevel_H264_Baseline_AutoLevel : kVTProfileLevel_HEVC_Main_AutoLevel
    }
}

struct StreamProfile: Codable, Equatable, Hashable {
    let codec: VideoCodec
    let width: UInt16
    let height: UInt16
    let fps: UInt8
}

func streamConfigurationCandidates(
    requested: StreamProfile,
    available: [StreamProfile]
) -> [StreamProfile] {
    var candidates = [requested]
    if requested.codec == .hevc {
        let fallback = StreamProfile(
            codec: .h264,
            width: requested.width,
            height: requested.height,
            fps: requested.fps
        )
        if available.contains(fallback) {
            candidates.append(fallback)
        }
    }
    return candidates
}
private struct VideoConfig: Encodable {
    let active_profile: StreamProfile
    let supported_profiles: [StreamProfile]
}



final class StreamManager: ObservableObject {
    struct DesktopEndpoint: Identifiable, Equatable {
        let name: String
        let host: String
        let port: UInt16

        var id: String { "\(host):\(port)" }
    }

    @Published private(set) var statusText = "Idle"
    @Published private(set) var isConnected = false
    @Published var selectedResolution: CaptureResolution = .p720
    @Published var selectedFps: Int32 = 30
    @Published var selectedCodec: VideoCodec = .h264
    @Published private(set) var isFrontCamera = false
    @Published private(set) var discoveredDesktops: [DesktopEndpoint] = []
    @Published private(set) var isDiscovering = false

    private let cameraController: CameraController
    private var encoder: VideoEncoder?
    private var connectionStatusTimer: Timer?
    private var cameraControlTimer: Timer?
    private var isStreaming = false
    private var cameraSwitchInProgress = false

    private var committedProfile = StreamProfile(
        codec: .h264,
        width: 1280,
        height: 720,
        fps: 30
    )
    private var availableProfiles: [StreamProfile] = []

    private var activeProfile: StreamProfile { committedProfile }
    private var supportedProfiles: [StreamProfile] { availableProfiles }


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
        cameraControlTimer?.invalidate()
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

        cameraController.requestAccessAndConfigure(
            resolution: selectedResolution,
            fps: selectedFps
        ) { [weak self] granted in
            guard let self else {
                return
            }

            guard granted else {
                DispatchQueue.main.async {
                    self.statusText = "Camera permission or requested format unavailable"
                    self.isConnected = false
                }
                return
            }

            self.cameraController.onSampleBuffer = { [weak self] sampleBuffer in
                self?.encoder?.encode(sampleBuffer: sampleBuffer)
            }
            self.availableProfiles = self.discoverSupportedProfiles()
            let dimensions = self.selectedResolution.dimensionsU16
            let startupProfile = StreamProfile(
                codec: self.selectedCodec,
                width: dimensions.width,
                height: dimensions.height,
                fps: UInt8(self.selectedFps)
            )
            guard
                self.availableProfiles.contains(startupProfile),
                let candidate = self.prepareEncoder(for: startupProfile)
            else {
                DispatchQueue.main.async {
                    self.statusText = "Selected camera/encoder profile is unavailable"
                }
                return
            }
            self.commitEncoder(candidate, profile: startupProfile)
            self.initializeTransport()
            self.cameraController.startSession()
            self.startConnectionStatusPolling()
            self.startCameraControlPolling()

            DispatchQueue.main.async {
                self.isStreaming = true
                self.isFrontCamera = self.cameraController.isUsingFrontCamera
                self.statusText = self.isConnected
                    ? self.streamingStatusText()
                    : self.disconnectedStatusText()
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
        stopCameraControlPolling()
        cameraSwitchInProgress = false

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

        reconfigureActiveCapture()
    }

    func setFrameRate(_ fps: Int32) {
        guard Self.supportedFrameRates.contains(fps), selectedFps != fps else {
            return
        }

        selectedFps = fps
        guard isStreaming else {
            return
        }

        reconfigureActiveCapture()
    }

    func setCodec(_ codec: VideoCodec) {
        guard selectedCodec != codec else { return }
        selectedCodec = codec
        guard isStreaming else { return }
        reconfigureActiveCapture()
    }

    func discoverDesktops() {
        guard !isDiscovering else {
            return
        }

        isDiscovering = true
        DispatchQueue.global(qos: .userInitiated).async { [weak self] in
            let endpoints = Self.readDiscoveredDesktops(timeoutMs: 3_000)
            DispatchQueue.main.async {
                self?.discoveredDesktops = endpoints
                self?.isDiscovering = false
                if endpoints.isEmpty {
                    self?.statusText = "No desktops found; scan the desktop QR code"
                }
            }
        }
    }

    func connect(to desktop: DesktopEndpoint) {
        endpointHost = desktop.host
        endpointPort = desktop.port

        if isStreaming {
            phonecam_transport_shutdown()
            let connected = initializeTransport()
            statusText = connected
                ? "Connected to \(desktop.name)"
                : "Unable to connect to \(desktop.name)"
        } else {
            startStreaming(host: desktop.host, port: desktop.port)
        }
    }

    private func reconfigureActiveCapture() {
        let dimensions = selectedResolution.dimensionsU16
        let profile = StreamProfile(
            codec: selectedCodec,
            width: dimensions.width,
            height: dimensions.height,
            fps: UInt8(selectedFps)
        )
        applyProfile(profile, requestID: 0, requirePeerSupport: true)
    }

    private func applyProfile(
        _ requested: StreamProfile,
        requestID: UInt32,
        requirePeerSupport: Bool
    ) {
        guard !cameraSwitchInProgress, availableProfiles.contains(requested) else {
            reportConfiguration(requestID: requestID, result: 1, profile: requested)
            return
        }
        if requirePeerSupport, isConnected,
           !phonecam_peer_supports_profile(
               requested.codec.nativeID,
               requested.width,
               requested.height,
               requested.fps
           ) {
            selectedResolution = CaptureResolution.matching(
                width: activeProfile.width,
                height: activeProfile.height
            ) ?? .p720
            selectedFps = Int32(activeProfile.fps)
            return
        }
        guard let resolution = CaptureResolution.matching(
            width: requested.width,
            height: requested.height
        ) else {
            reportConfiguration(requestID: requestID, result: 1, profile: requested)
            return
        }

        cameraSwitchInProgress = true
        let previous = activeProfile
        var applied = requested
        var candidate: VideoEncoder?
        for possible in streamConfigurationCandidates(
            requested: requested,
            available: availableProfiles
        ) {
            if let prepared = prepareEncoder(for: possible) {
                candidate = prepared
                applied = possible
                break
            }
        }
        guard let candidate else {
            availableProfiles.removeAll { $0 == requested }
            updateCapabilities()
            reportConfiguration(requestID: requestID, result: 2, profile: requested)
            cameraSwitchInProgress = false
            return
        }

        cameraController.stopSession()
        cameraController.requestAccessAndConfigure(
            resolution: resolution,
            fps: Int32(applied.fps)
        ) { [weak self] success in
            guard let self else { return }
            defer { self.cameraSwitchInProgress = false }
            guard success else {
                candidate.stop()
                self.availableProfiles.removeAll { $0 == requested }
                self.updateCapabilities()
                self.reportConfiguration(requestID: requestID, result: 2, profile: requested)
                self.restoreCapture(profile: previous)
                return
            }
            self.selectedResolution = resolution
            self.selectedFps = Int32(applied.fps)
            self.selectedCodec = applied.codec
            self.commitEncoder(candidate, profile: applied)
            self.reportConfiguration(requestID: requestID, result: 0, profile: applied)
            self.cameraController.startSession()
            self.encoder?.requestKeyFrame()
            self.statusText = self.streamingStatusText()
        }
    }

    private func restoreCapture(profile: StreamProfile) {
        guard let resolution = CaptureResolution.matching(
            width: profile.width,
            height: profile.height
        ) else { return }
        selectedResolution = resolution
        selectedFps = Int32(profile.fps)
        selectedCodec = profile.codec
        cameraController.requestAccessAndConfigure(
            resolution: resolution,
            fps: Int32(profile.fps)
        ) { [weak self] success in
            guard let self else { return }
            guard success else {
                self.encoder?.stop()
                self.isStreaming = false
                self.statusText = "Terminal stream failure: unable to restore previous configuration"
                phonecam_transport_shutdown()
                return
            }
            self.cameraController.startSession()
            self.encoder?.requestKeyFrame()
        }
    }

    private func reportConfiguration(
        requestID: UInt32,
        result: UInt8,
        profile: StreamProfile
    ) {
        guard isConnected else { return }
        _ = phonecam_report_stream_configuration(
            requestID,
            result,
            profile.codec.nativeID,
            profile.width,
            profile.height,
            profile.fps
        )
    }

    private func updateCapabilities() {
        guard let data = try? JSONEncoder().encode(availableProfiles),
              let json = String(data: data, encoding: .utf8)
        else { return }
        json.withCString { _ = phonecam_update_video_capabilities($0) }
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

    private func prepareEncoder(for profile: StreamProfile) -> VideoEncoder? {
        let candidate = VideoEncoder(
            codec: profile.codec,
            width: Int32(profile.width),
            height: Int32(profile.height),
            fps: Int32(profile.fps),
            bitrate: Self.bitrate(for: profile)
        )
        candidate.onEncodedNALUnit = { [weak self] nalUnit, ptsUs, isKeyframe in
            guard let self else { return }
            let active = self.activeProfile
            let accepted = nalUnit.withUnsafeBytes { rawBuffer -> Bool in
                guard let baseAddress = rawBuffer.baseAddress?.assumingMemoryBound(to: UInt8.self)
                else { return false }
                return phonecam_send_video_frame(
                    baseAddress,
                    rawBuffer.count,
                    ptsUs,
                    active.codec.nativeID,
                    active.width,
                    active.height,
                    isKeyframe
                )
            }
            if !accepted && isKeyframe { self.encoder?.requestKeyFrame() }
        }
        candidate.onFatalError = { [weak self] error in
            DispatchQueue.main.async {
                self?.statusText = "Encoder error: \(error)"
            }
        }
        do {
            try candidate.start()
            return candidate
        } catch {
            candidate.stop()
            return nil
        }
    }

    private func commitEncoder(_ candidate: VideoEncoder, profile: StreamProfile) {
        let previous = encoder
        committedProfile = profile
        encoder = candidate
        previous?.stop()
    }


    @discardableResult
    private func initializeTransport() -> Bool {
        guard
            let configData = try? JSONEncoder().encode(
                VideoConfig(
                    active_profile: activeProfile,
                    supported_profiles: supportedProfiles
                )
            ),
            let configJSON = String(data: configData, encoding: .utf8)
        else {
            return false
        }
        let initialized = endpointHost.withCString { hostCString in
            configJSON.withCString { configCString in
                phonecam_transport_init(hostCString, endpointPort, configCString)
            }
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

    private static func readDiscoveredDesktops(timeoutMs: UInt32) -> [DesktopEndpoint] {
        guard let ptr = phonecam_discover_desktops(timeoutMs) else {
            return []
        }
        defer { phonecam_string_free(ptr) }

        return String(cString: ptr)
            .split(whereSeparator: { $0.isNewline })
            .compactMap { record in
                let parts = record.split(separator: "|", maxSplits: 3, omittingEmptySubsequences: false)
                guard
                    parts.count == 4,
                    let port = UInt16(parts[2])
                else {
                    return nil
                }

                let name = String(parts[0]).trimmingCharacters(in: .whitespacesAndNewlines)
                let host = String(parts[1]).trimmingCharacters(in: .whitespacesAndNewlines)
                guard !host.isEmpty else {
                    return nil
                }

                return DesktopEndpoint(
                    name: name.isEmpty ? "PhoneCam Desktop" : name,
                    host: host,
                    port: port
                )
            }
    }

    private func streamingStatusText() -> String {
        "Streaming (\(selectedResolution.rawValue) @ \(selectedFps) FPS, \(isFrontCamera ? "front" : "back") camera)"
    }

    private func disconnectedStatusText() -> String {
        "Camera active, transport disconnected (\(isFrontCamera ? "front" : "back") camera)"
    }

    private func startCameraControlPolling() {
        stopCameraControlPolling()

        let timer = Timer.scheduledTimer(withTimeInterval: Self.cameraControlPollInterval, repeats: true) { [weak self] _ in
            self?.pollCameraControlCommand()
        }

        RunLoop.main.add(timer, forMode: .common)
        cameraControlTimer = timer
    }

    private func stopCameraControlPolling() {
        cameraControlTimer?.invalidate()
        cameraControlTimer = nil
    }

    private func pollCameraControlCommand() {
        guard isStreaming, !cameraSwitchInProgress, let pointer = phonecam_poll_control_command_json() else {
            return
        }
        defer { phonecam_string_free(pointer) }
        guard
            let data = String(cString: pointer).data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let type = object["type"] as? String
        else {
            return
        }
        switch type {
        case "switch_camera":
            if let front = object["front"] as? Bool {
                handleRemoteCameraSwitch(toFront: front)
            }
        case "request_keyframe":
            encoder?.requestKeyFrame()
        case "configure_stream":
            guard
                let requestNumber = object["request_id"] as? NSNumber,
                let profileObject = object["profile"] as? [String: Any],
                let codecLiteral = profileObject["codec"] as? String,
                let codec = VideoCodec(rawValue: codecLiteral),
                let width = (profileObject["width"] as? NSNumber)?.uint16Value,
                let height = (profileObject["height"] as? NSNumber)?.uint16Value,
                let fps = (profileObject["fps"] as? NSNumber)?.uint8Value
            else {
                return
            }
            handleRemoteStreamConfiguration(
                requestID: requestNumber.uint32Value,
                profile: StreamProfile(codec: codec, width: width, height: height, fps: fps)
            )
        default:
            return
        }
    }

    private func handleRemoteStreamConfiguration(requestID: UInt32, profile: StreamProfile) {
        guard
            CaptureResolution.matching(width: profile.width, height: profile.height) != nil,
            Self.supportedFrameRates.contains(Int32(profile.fps)),
            supportedProfiles.contains(profile)
        else {
            reportConfiguration(requestID: requestID, result: 1, profile: profile)
            statusText = "Desktop requested unsupported stream configuration"
            return
        }
        if profile == activeProfile {
            reportConfiguration(requestID: requestID, result: 0, profile: profile)
            encoder?.requestKeyFrame()
            return
        }
        applyProfile(profile, requestID: requestID, requirePeerSupport: false)
    }

    private func handleRemoteCameraSwitch(toFront: Bool) {
        guard isStreaming, !cameraSwitchInProgress else {
            return
        }

        cameraSwitchInProgress = true
        statusText = "Switching to \(toFront ? "front" : "back") camera…"

        cameraController.stopSession()

        cameraController.switchCamera(toFront: toFront) { [weak self] success, actualFront in
            guard let self else {
                return
            }

            defer {
                self.cameraSwitchInProgress = false
            }

            self.cameraController.startSession()
            self.encoder?.requestKeyFrame()
            self.availableProfiles =
                self.discoverSupportedProfiles(
                    position: actualFront ? .front : .back
                )
            self.updateCapabilities()

            self.isFrontCamera = actualFront

            guard success else {
                self.statusText = "Camera switch failed"
                return
            }

            if actualFront != toFront {
                self.statusText = "Requested \(toFront ? "front" : "back") camera unavailable; using \(actualFront ? "front" : "back")"
                return
            }

            self.statusText = self.isConnected ? self.streamingStatusText() : self.disconnectedStatusText()
        }
    }

    private func startConnectionStatusPolling() {
        connectionStatusTimer?.invalidate()
        let timer = Timer.scheduledTimer(withTimeInterval: 0.5, repeats: true) { [weak self] _ in
            guard let self else { return }
            let connected = phonecam_transport_is_connected()
            guard connected != self.isConnected else { return }
            self.isConnected = connected
            if connected {
                self.statusText = self.streamingStatusText()
            } else if self.isStreaming {
                self.statusText = self.disconnectedStatusText()
            }
        }
        RunLoop.main.add(timer, forMode: .common)
        connectionStatusTimer = timer
    }

    private func discoverSupportedProfiles(
        position: AVCaptureDevice.Position = .back
    ) -> [StreamProfile] {
        cameraController.supportedCaptureProfiles(position: position)
            .flatMap { capture in
                VideoCodec.allCases.compactMap { codec -> StreamProfile? in
                    let dimensions = capture.resolution.dimensionsU16
                    let profile = StreamProfile(
                        codec: codec,
                        width: dimensions.width,
                        height: dimensions.height,
                        fps: UInt8(capture.fps)
                    )
                    let probe = VideoEncoder(
                        codec: codec,
                        width: Int32(profile.width),
                        height: Int32(profile.height),
                        fps: Int32(profile.fps),
                        bitrate: Self.bitrate(for: profile)
                    )
                    do {
                        try probe.start()
                        probe.stop()
                        return profile
                    } catch {
                        probe.stop()
                        return nil
                    }
                }
            }
            .sorted {
                (Int($0.width) * Int($0.height), $0.fps, $0.codec.nativeID)
                    < (Int($1.width) * Int($1.height), $1.fps, $1.codec.nativeID)
            }
    }

    static func bitrate(for profile: StreamProfile) -> Int {
        guard
            let resolution = CaptureResolution.matching(
                width: profile.width,
                height: profile.height
            ),
            let index = [15, 30, 60].firstIndex(of: Int(profile.fps))
        else { preconditionFailure("Invalid stream profile") }
        let values: [Int]
        switch (profile.codec, resolution) {
        case (.h264, .p480): values = [1_000_000, 2_000_000, 3_000_000]
        case (.h264, .p720): values = [2_500_000, 4_000_000, 7_000_000]
        case (.h264, .p1080): values = [4_000_000, 8_000_000, 12_000_000]
        case (.h264, .p1440): values = [8_000_000, 16_000_000, 24_000_000]
        case (.h264, .p2160): values = [16_000_000, 35_000_000, 50_000_000]
        case (.hevc, .p480): values = [750_000, 1_250_000, 2_000_000]
        case (.hevc, .p720): values = [1_500_000, 2_500_000, 4_500_000]
        case (.hevc, .p1080): values = [2_500_000, 5_000_000, 8_000_000]
        case (.hevc, .p1440): values = [5_000_000, 9_000_000, 15_000_000]
        case (.hevc, .p2160): values = [10_000_000, 20_000_000, 32_000_000]
        }
        return values[index]
    }
    private static let supportedFrameRates: Set<Int32> = [15, 30, 60]
}


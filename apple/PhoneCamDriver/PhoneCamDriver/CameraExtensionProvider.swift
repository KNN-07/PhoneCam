import CoreMedia
import CoreMediaIO
import Foundation
import IOKit.audio
import os.log

private let providerLogger = Logger(
    subsystem: "com.phonecam.driver.cameraextension",
    category: "Provider"
)

struct CameraExtensionConfiguration {
    let appGroupIdentifier: String
    let deviceID: UUID
    let streamID: UUID

    static func loadFromMainBundle() -> CameraExtensionConfiguration {
        let info = Bundle.main.infoDictionary ?? [:]

        let fallbackAppGroup = "group.com.phonecam.shared"
        let fallbackDeviceID = "7E9F5417-21C7-430B-9018-486B7A1C64C0"
        let fallbackStreamID = "8A78CCEC-C80C-4D42-B51F-C755572164A2"

        let appGroupIdentifier = (info["PhoneCamAppGroupIdentifier"] as? String)?.trimmingCharacters(
            in: .whitespacesAndNewlines
        )
            .nonEmpty
            ?? fallbackAppGroup

        let deviceID = UUID(
            uuidString: (info["PhoneCamDeviceUUID"] as? String)?.trimmingCharacters(
                in: .whitespacesAndNewlines
            )
                .nonEmpty
                ?? fallbackDeviceID
        ) ?? UUID(uuidString: fallbackDeviceID)!

        let streamID = UUID(
            uuidString: (info["PhoneCamStreamUUID"] as? String)?.trimmingCharacters(
                in: .whitespacesAndNewlines
            )
                .nonEmpty
                ?? fallbackStreamID
        ) ?? UUID(uuidString: fallbackStreamID)!

        return CameraExtensionConfiguration(
            appGroupIdentifier: appGroupIdentifier,
            deviceID: deviceID,
            streamID: streamID
        )
    }
}

final class CameraExtensionProviderSource: NSObject, CMIOExtensionProviderSource {
    private(set) var provider: CMIOExtensionProvider!

    private let deviceSource: PhoneCamDeviceSource

    init(configuration: CameraExtensionConfiguration, clientQueue: DispatchQueue?) {
        self.deviceSource = PhoneCamDeviceSource(configuration: configuration, localizedName: "PhoneCam")

        super.init()

        provider = CMIOExtensionProvider(source: self, clientQueue: clientQueue)

        do {
            try provider.addDevice(deviceSource.device)
        } catch {
            fatalError("Unable to register PhoneCam device: \(error.localizedDescription)")
        }
    }

    func connect(to client: CMIOExtensionClient) throws {
        providerLogger.debug("Client connected to PhoneCam extension")
        _ = client
    }

    func disconnect(from client: CMIOExtensionClient) {
        providerLogger.debug("Client disconnected from PhoneCam extension")
        _ = client
    }

    var availableProperties: Set<CMIOExtensionProperty> {
        [.providerName, .providerManufacturer]
    }

    func providerProperties(
        forProperties properties: Set<CMIOExtensionProperty>
    ) throws -> CMIOExtensionProviderProperties {
        let providerProperties = CMIOExtensionProviderProperties(dictionary: [:])

        if properties.contains(.providerName) {
            providerProperties.name = "PhoneCam Camera Extension"
        }

        if properties.contains(.providerManufacturer) {
            providerProperties.manufacturer = "PhoneCam"
        }

        return providerProperties
    }

    func setProviderProperties(_ providerProperties: CMIOExtensionProviderProperties) throws {
        _ = providerProperties
    }
}

final class PhoneCamDeviceSource: NSObject, CMIOExtensionDeviceSource {
    private(set) var device: CMIOExtensionDevice!

    private let logger = Logger(
        subsystem: "com.phonecam.driver.cameraextension",
        category: "Device"
    )

    private let outputQueue = DispatchQueue(
        label: "com.phonecam.driver.cameraextension.output",
        qos: .userInteractive
    )
    private let frameBufferQueue = FrameBufferQueue(capacity: 8)
    private let ipcReceiver: IPCReceiver

    private var streamSource: PhoneCamStreamSource!
    private var streamingClientCount: UInt32 = 0
    private var streamTimer: DispatchSourceTimer?
    private var frameDuration = CMTime(value: 1, timescale: 30)
    private var lastDeliveredBuffer: CMSampleBuffer?

    init(configuration: CameraExtensionConfiguration, localizedName: String) {
        self.ipcReceiver = IPCReceiver(appGroupIdentifier: configuration.appGroupIdentifier)

        super.init()

        device = CMIOExtensionDevice(
            localizedName: localizedName,
            deviceID: configuration.deviceID,
            legacyDeviceID: configuration.deviceID.uuidString,
            source: self
        )

        streamSource = PhoneCamStreamSource(
            localizedName: "PhoneCam Video",
            streamID: configuration.streamID,
            streamFormats: Self.createStreamFormats(),
            device: device
        )

        do {
            try device.addStream(streamSource.stream)
        } catch {
            fatalError("Unable to add PhoneCam stream: \(error.localizedDescription)")
        }

        ipcReceiver.onSampleBuffer = { [weak self] sampleBuffer in
            self?.frameBufferQueue.enqueue(sampleBuffer)
        }

        do {
            try ipcReceiver.start()
        } catch {
            logger.error("Failed to start IPC receiver: \(String(describing: error), privacy: .public)")
        }
    }

    deinit {
        streamTimer?.cancel()
        ipcReceiver.stop()
    }

    var availableProperties: Set<CMIOExtensionProperty> {
        [.deviceTransportType, .deviceModel]
    }

    func deviceProperties(
        forProperties properties: Set<CMIOExtensionProperty>
    ) throws -> CMIOExtensionDeviceProperties {
        let deviceProperties = CMIOExtensionDeviceProperties(dictionary: [:])

        if properties.contains(.deviceTransportType) {
            deviceProperties.transportType = kIOAudioDeviceTransportTypeVirtual
        }

        if properties.contains(.deviceModel) {
            deviceProperties.model = "PhoneCam Virtual Camera"
        }

        return deviceProperties
    }

    func setDeviceProperties(_ deviceProperties: CMIOExtensionDeviceProperties) throws {
        _ = deviceProperties
    }

    func startStreaming() {
        outputQueue.async {
            self.streamingClientCount += 1

            guard self.streamTimer == nil else {
                return
            }

            self.rebuildStreamTimerLocked()
        }
    }

    func stopStreaming() {
        outputQueue.async {
            if self.streamingClientCount > 1 {
                self.streamingClientCount -= 1
                return
            }

            self.streamingClientCount = 0
            self.streamTimer?.cancel()
            self.streamTimer = nil
            self.lastDeliveredBuffer = nil
            self.frameBufferQueue.removeAll()
        }
    }

    func setFrameDuration(_ newFrameDuration: CMTime) {
        outputQueue.async {
            guard
                newFrameDuration.isValid,
                !newFrameDuration.isIndefinite,
                newFrameDuration.seconds > 0
            else {
                return
            }

            self.frameDuration = newFrameDuration

            guard self.streamingClientCount > 0 else {
                return
            }

            self.rebuildStreamTimerLocked()
        }
    }

    private func rebuildStreamTimerLocked() {
        streamTimer?.cancel()
        streamTimer = nil

        let interval = max(
            1.0 / 120.0,
            (frameDuration.isValid && frameDuration.seconds > 0) ? frameDuration.seconds : (1.0 / 30.0)
        )

        let timer = DispatchSource.makeTimerSource(flags: .strict, queue: outputQueue)
        timer.schedule(deadline: .now(), repeating: interval, leeway: .milliseconds(2))
        timer.setEventHandler { [weak self] in
            self?.deliverFrameIfAvailable()
        }
        timer.resume()

        streamTimer = timer
    }

    private func deliverFrameIfAvailable() {
        guard streamingClientCount > 0 else {
            return
        }

        guard let sampleBuffer = frameBufferQueue.dequeue() ?? lastDeliveredBuffer else {
            return
        }

        lastDeliveredBuffer = sampleBuffer

        streamSource.stream.send(
            sampleBuffer,
            discontinuity: [],
            hostTimeInNanoseconds: Self.hostTimeInNanoseconds(for: sampleBuffer)
        )
    }

    private static func hostTimeInNanoseconds(for sampleBuffer: CMSampleBuffer) -> UInt64 {
        let pts = sampleBuffer.presentationTimeStamp

        if pts.isValid, !pts.isIndefinite {
            let nanoseconds = CMTimeConvertScale(pts, timescale: 1_000_000_000, method: .default)
            if nanoseconds.value > 0 {
                return UInt64(nanoseconds.value)
            }
        }

        let now = CMClockGetTime(CMClockGetHostTimeClock())
        return UInt64(max(0, now.seconds * 1_000_000_000))
    }

    private static func createStreamFormats() -> [CMIOExtensionStreamFormat] {
        let dimensions = [
            CMVideoDimensions(width: 640, height: 480),
            CMVideoDimensions(width: 1280, height: 720),
            CMVideoDimensions(width: 1920, height: 1080),
        ]

        return dimensions.map { dimensions in
            var formatDescription: CMFormatDescription?
            let status = CMVideoFormatDescriptionCreate(
                allocator: kCFAllocatorDefault,
                codecType: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
                width: dimensions.width,
                height: dimensions.height,
                extensions: nil,
                formatDescriptionOut: &formatDescription
            )

            guard status == noErr, let formatDescription else {
                fatalError("Failed to create (dimensions.width)x(dimensions.height) stream format")
            }

            return CMIOExtensionStreamFormat(
                formatDescription: formatDescription,
                maxFrameDuration: CMTime(value: 1, timescale: 15),
                minFrameDuration: CMTime(value: 1, timescale: 60),
                validFrameDurations: nil
            )
        }
    }
}

private extension String {
    var nonEmpty: String? {
        isEmpty ? nil : self
    }
}

import CoreMedia
import CoreMediaIO
import Foundation
import os.log

final class PhoneCamStreamSource: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    let clientQueue: DispatchQueue

    private let logger = Logger(
        subsystem: "com.phonecam.driver.cameraextension",
        category: "Stream"
    )

    private let device: CMIOExtensionDevice
    private let streamFormats: [CMIOExtensionStreamFormat]

    private var configuredFrameDuration = CMTime(value: 1, timescale: 30)

    init(
        localizedName: String,
        streamID: UUID,
        streamFormats: [CMIOExtensionStreamFormat],
        device: CMIOExtensionDevice,
        clientQueue: DispatchQueue = DispatchQueue(
            label: "com.phonecam.driver.cameraextension.streamsource",
            qos: .userInitiated
        )
    ) {
        self.device = device
        precondition(!streamFormats.isEmpty, "PhoneCam must expose at least one stream format")
        self.streamFormats = streamFormats
        self.clientQueue = clientQueue

        super.init()

        stream = CMIOExtensionStream(
            localizedName: localizedName,
            streamID: streamID,
            direction: .source,
            clockType: .hostTime,
            source: self
        )
    }

    var formats: [CMIOExtensionStreamFormat] {
        streamFormats
    }

    var activeFormatIndex: Int = 1 {
        didSet {
            if !streamFormats.indices.contains(activeFormatIndex) {
                logger.error("Unsupported PhoneCam stream format index: \(self.activeFormatIndex)")
                activeFormatIndex = min(1, streamFormats.count - 1)
            }
        }
    }

    var availableProperties: Set<CMIOExtensionProperty> {
        [.streamActiveFormatIndex, .streamFrameDuration]
    }

    func streamProperties(
        forProperties properties: Set<CMIOExtensionProperty>
    ) throws -> CMIOExtensionStreamProperties {
        let streamProperties = CMIOExtensionStreamProperties(dictionary: [:])

        if properties.contains(.streamActiveFormatIndex) {
            streamProperties.activeFormatIndex = activeFormatIndex
        }

        if properties.contains(.streamFrameDuration) {
            streamProperties.frameDuration = configuredFrameDuration
        }

        return streamProperties
    }

    func setStreamProperties(_ streamProperties: CMIOExtensionStreamProperties) throws {
        if let requestedActiveFormatIndex = streamProperties.activeFormatIndex {
            activeFormatIndex = requestedActiveFormatIndex
        }

        if let requestedFrameDuration = streamProperties.frameDuration,
           requestedFrameDuration.isValid,
           !requestedFrameDuration.isIndefinite,
           requestedFrameDuration.seconds > 0
        {
            configuredFrameDuration = requestedFrameDuration

            if let deviceSource = device.source as? PhoneCamDeviceSource {
                deviceSource.setFrameDuration(requestedFrameDuration)
            }
        }
    }

    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool {
        _ = client
        return true
    }

    func startStream() throws {
        guard let deviceSource = device.source as? PhoneCamDeviceSource else {
            fatalError("Unexpected stream owner for PhoneCam stream")
        }

        deviceSource.startStreaming()
    }

    func stopStream() throws {
        guard let deviceSource = device.source as? PhoneCamDeviceSource else {
            fatalError("Unexpected stream owner for PhoneCam stream")
        }

        deviceSource.stopStreaming()
    }
}

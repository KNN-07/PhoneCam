import CoreMedia
import CoreVideo
import Darwin
import Foundation
import os.log

enum IPCReceiverError: Error {
    case appGroupContainerUnavailable(String)
    case socketPathTooLong(String)
    case socketCreationFailed(Int32)
    case bindFailed(Int32)
    case listenFailed(Int32)
}

private struct NV12FramePacket {
    let width: Int
    let height: Int
    let timestampNanoseconds: UInt64
    let payload: Data
}

final class IPCReceiver {
    var onSampleBuffer: ((CMSampleBuffer) -> Void)?

    private let logger = Logger(
        subsystem: "com.phonecam.driver.cameraextension",
        category: "IPC"
    )

    private let appGroupIdentifier: String
    private let queue = DispatchQueue(
        label: "com.phonecam.driver.cameraextension.ipc",
        qos: .userInitiated
    )

    private var serverFileDescriptor: Int32 = -1
    private var serverReadSource: DispatchSourceRead?
    private var socketURL: URL?
    private var clients: [Int32: IPCClientConnection] = [:]

    init(appGroupIdentifier: String) {
        self.appGroupIdentifier = appGroupIdentifier
    }

    deinit {
        stop()
    }

    func start() throws {
        guard serverFileDescriptor == -1 else {
            return
        }

        let socketURL = try Self.makeSocketURL(forAppGroupIdentifier: appGroupIdentifier)
        try Self.removeSocketFileIfPresent(at: socketURL)

        let socketFD = socket(AF_UNIX, SOCK_STREAM, 0)
        guard socketFD >= 0 else {
            throw IPCReceiverError.socketCreationFailed(errno)
        }

        var reuseAddress: Int32 = 1
        _ = setsockopt(
            socketFD,
            SOL_SOCKET,
            SO_REUSEADDR,
            &reuseAddress,
            socklen_t(MemoryLayout<Int32>.size)
        )

        let serverFlags = fcntl(socketFD, F_GETFL, 0)
        if serverFlags >= 0 {
            _ = fcntl(socketFD, F_SETFL, serverFlags | O_NONBLOCK)
        }

        var address = sockaddr_un()
        address.sun_family = sa_family_t(AF_UNIX)

        let socketPathCString = socketURL.path.utf8CString
        let maxPathLength = MemoryLayout.size(ofValue: address.sun_path)
        guard socketPathCString.count <= maxPathLength else {
            close(socketFD)
            throw IPCReceiverError.socketPathTooLong(socketURL.path)
        }

        let socketPathBytes = socketPathCString.map { UInt8(bitPattern: $0) }
        withUnsafeMutableBytes(of: &address.sun_path) { destination in
            destination.copyBytes(from: socketPathBytes)
        }

        let addressLength = socklen_t(MemoryLayout<sa_family_t>.size + socketPathCString.count)
        let bindStatus = withUnsafePointer(to: &address) { pointer in
            pointer.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                bind(socketFD, $0, addressLength)
            }
        }

        guard bindStatus == 0 else {
            let bindError = errno
            close(socketFD)
            throw IPCReceiverError.bindFailed(bindError)
        }

        guard listen(socketFD, 4) == 0 else {
            let listenError = errno
            close(socketFD)
            throw IPCReceiverError.listenFailed(listenError)
        }

        self.serverFileDescriptor = socketFD
        self.socketURL = socketURL

        let serverSource = DispatchSource.makeReadSource(fileDescriptor: socketFD, queue: queue)
        serverSource.setEventHandler { [weak self] in
            self?.acceptConnections()
        }
        serverSource.setCancelHandler {
            close(socketFD)
        }

        serverReadSource = serverSource
        serverSource.resume()
    }

    func stop() {
        queue.async {
            self.clients.values.forEach { client in
                client.readSource?.cancel()
            }
            self.clients.removeAll()

            self.serverReadSource?.cancel()
            self.serverReadSource = nil
            self.serverFileDescriptor = -1

            if let socketURL = self.socketURL {
                try? Self.removeSocketFileIfPresent(at: socketURL)
                self.socketURL = nil
            }
        }
    }

    private func acceptConnections() {
        guard serverFileDescriptor >= 0 else {
            return
        }

        while true {
            let clientFD = accept(serverFileDescriptor, nil, nil)
            if clientFD < 0 {
                if errno == EWOULDBLOCK || errno == EAGAIN {
                    return
                }

                logger.error("Failed to accept IPC client, errno=\(errno)")
                return
            }

            configureClientSocket(clientFD)
        }
    }

    private func configureClientSocket(_ fileDescriptor: Int32) {
        let flags = fcntl(fileDescriptor, F_GETFL, 0)
        if flags >= 0 {
            _ = fcntl(fileDescriptor, F_SETFL, flags | O_NONBLOCK)
        }

        let client = IPCClientConnection(fileDescriptor: fileDescriptor)
        let readSource = DispatchSource.makeReadSource(fileDescriptor: fileDescriptor, queue: queue)
        client.readSource = readSource

        readSource.setEventHandler { [weak self, weak client] in
            guard let self, let client else {
                return
            }

            self.readFromClient(client)
        }

        readSource.setCancelHandler {
            close(fileDescriptor)
        }

        clients[fileDescriptor] = client
        readSource.resume()
    }

    private func readFromClient(_ client: IPCClientConnection) {
        var buffer = [UInt8](repeating: 0, count: 64 * 1024)

        while true {
            let readCount: ssize_t = buffer.withUnsafeMutableBytes { rawBuffer in
                guard let baseAddress = rawBuffer.baseAddress else {
                    return -1
                }

                return recv(client.fileDescriptor, baseAddress, rawBuffer.count, 0)
            }

            if readCount > 0 {
                client.pendingData.append(contentsOf: buffer.prefix(Int(readCount)))
                processPendingData(for: client)
                continue
            }

            if readCount == 0 {
                closeClient(fileDescriptor: client.fileDescriptor)
                return
            }

            if errno == EWOULDBLOCK || errno == EAGAIN {
                return
            }

            logger.error("IPC client read failed, errno=\(errno)")
            closeClient(fileDescriptor: client.fileDescriptor)
            return
        }
    }

    private func processPendingData(for client: IPCClientConnection) {
        while client.pendingData.count >= 16 {
            let width = Int(readUInt32LE(from: client.pendingData, at: 0))
            let height = Int(readUInt32LE(from: client.pendingData, at: 4))
            let timestampNanoseconds = readUInt64LE(from: client.pendingData, at: 8)

            guard let payloadSize = Self.nv12PayloadSize(width: width, height: height) else {
                client.pendingData.removeFirst(16)
                continue
            }

            let packetSize = 16 + payloadSize
            guard client.pendingData.count >= packetSize else {
                return
            }

            let payload = Data(client.pendingData[16..<packetSize])
            client.pendingData.removeFirst(packetSize)

            let packet = NV12FramePacket(
                width: width,
                height: height,
                timestampNanoseconds: timestampNanoseconds,
                payload: payload
            )

            guard let sampleBuffer = makeSampleBuffer(from: packet) else {
                continue
            }

            onSampleBuffer?(sampleBuffer)
        }
    }

    private func makeSampleBuffer(from packet: NV12FramePacket) -> CMSampleBuffer? {
        let attributes: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            kCVPixelBufferWidthKey: packet.width,
            kCVPixelBufferHeightKey: packet.height,
            kCVPixelBufferIOSurfacePropertiesKey: [:],
        ]

        var pixelBuffer: CVPixelBuffer?
        let pixelBufferStatus = CVPixelBufferCreate(
            kCFAllocatorDefault,
            packet.width,
            packet.height,
            kCVPixelFormatType_420YpCbCr8BiPlanarFullRange,
            attributes as CFDictionary,
            &pixelBuffer
        )

        guard pixelBufferStatus == kCVReturnSuccess, let pixelBuffer else {
            logger.error("CVPixelBufferCreate failed with status \(pixelBufferStatus)")
            return nil
        }

        CVPixelBufferLockBaseAddress(pixelBuffer, [])
        defer {
            CVPixelBufferUnlockBaseAddress(pixelBuffer, [])
        }

        guard copyNV12(payload: packet.payload, width: packet.width, height: packet.height, into: pixelBuffer) else {
            return nil
        }

        var formatDescription: CMVideoFormatDescription?
        let formatStatus = CMVideoFormatDescriptionCreateForImageBuffer(
            allocator: kCFAllocatorDefault,
            imageBuffer: pixelBuffer,
            formatDescriptionOut: &formatDescription
        )

        guard formatStatus == noErr, let formatDescription else {
            logger.error("CMVideoFormatDescriptionCreateForImageBuffer failed with status \(formatStatus)")
            return nil
        }

        var timingInfo = CMSampleTimingInfo(
            duration: .invalid,
            presentationTimeStamp: Self.presentationTimestamp(fromNanoseconds: packet.timestampNanoseconds),
            decodeTimeStamp: .invalid
        )

        var sampleBuffer: CMSampleBuffer?
        let sampleStatus = CMSampleBufferCreateForImageBuffer(
            allocator: kCFAllocatorDefault,
            imageBuffer: pixelBuffer,
            dataReady: true,
            makeDataReadyCallback: nil,
            refcon: nil,
            formatDescription: formatDescription,
            sampleTiming: &timingInfo,
            sampleBufferOut: &sampleBuffer
        )

        guard sampleStatus == noErr, let sampleBuffer else {
            logger.error("CMSampleBufferCreateForImageBuffer failed with status \(sampleStatus)")
            return nil
        }

        return sampleBuffer
    }

    private func copyNV12(payload: Data, width: Int, height: Int, into pixelBuffer: CVPixelBuffer) -> Bool {
        guard CVPixelBufferGetPlaneCount(pixelBuffer) >= 2 else {
            return false
        }

        guard
            let yDestination = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 0),
            let uvDestination = CVPixelBufferGetBaseAddressOfPlane(pixelBuffer, 1)
        else {
            return false
        }

        let yDestinationRowBytes = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 0)
        let uvDestinationRowBytes = CVPixelBufferGetBytesPerRowOfPlane(pixelBuffer, 1)

        return payload.withUnsafeBytes { payloadBuffer in
            guard let sourceBase = payloadBuffer.baseAddress else {
                return false
            }

            for row in 0..<height {
                let source = sourceBase.advanced(by: row * width)
                let destination = yDestination.advanced(by: row * yDestinationRowBytes)
                memcpy(destination, source, min(width, yDestinationRowBytes))
            }

            let uvOffset = width * height
            for row in 0..<(height / 2) {
                let source = sourceBase.advanced(by: uvOffset + row * width)
                let destination = uvDestination.advanced(by: row * uvDestinationRowBytes)
                memcpy(destination, source, min(width, uvDestinationRowBytes))
            }

            return true
        }
    }

    private func closeClient(fileDescriptor: Int32) {
        guard let client = clients.removeValue(forKey: fileDescriptor) else {
            return
        }

        client.readSource?.cancel()
    }

    private static func makeSocketURL(forAppGroupIdentifier appGroupIdentifier: String) throws -> URL {
        guard
            let containerURL = FileManager.default
                .containerURL(forSecurityApplicationGroupIdentifier: appGroupIdentifier)
        else {
            throw IPCReceiverError.appGroupContainerUnavailable(appGroupIdentifier)
        }

        return containerURL.appendingPathComponent("phonecam.sock", isDirectory: false)
    }

    private static func removeSocketFileIfPresent(at url: URL) throws {
        if FileManager.default.fileExists(atPath: url.path) {
            try FileManager.default.removeItem(at: url)
        }
    }

    private static func nv12PayloadSize(width: Int, height: Int) -> Int? {
        guard width > 0, height > 0 else {
            return nil
        }

        guard width % 2 == 0, height % 2 == 0 else {
            return nil
        }

        let lumaSamples = Int64(width) * Int64(height)
        let totalSamples = lumaSamples + (lumaSamples / 2)

        guard totalSamples > 0, totalSamples <= Int64(Int.max) else {
            return nil
        }

        return Int(totalSamples)
    }

    private static func presentationTimestamp(fromNanoseconds nanoseconds: UInt64) -> CMTime {
        guard nanoseconds > 0 else {
            return CMClockGetTime(CMClockGetHostTimeClock())
        }

        return CMTime(value: Int64(nanoseconds), timescale: 1_000_000_000)
    }

    private func readUInt32LE(from data: Data, at offset: Int) -> UInt32 {
        UInt32(data[offset])
            | (UInt32(data[offset + 1]) << 8)
            | (UInt32(data[offset + 2]) << 16)
            | (UInt32(data[offset + 3]) << 24)
    }

    private func readUInt64LE(from data: Data, at offset: Int) -> UInt64 {
        UInt64(data[offset])
            | (UInt64(data[offset + 1]) << 8)
            | (UInt64(data[offset + 2]) << 16)
            | (UInt64(data[offset + 3]) << 24)
            | (UInt64(data[offset + 4]) << 32)
            | (UInt64(data[offset + 5]) << 40)
            | (UInt64(data[offset + 6]) << 48)
            | (UInt64(data[offset + 7]) << 56)
    }
}

private final class IPCClientConnection {
    let fileDescriptor: Int32
    var pendingData = Data()
    var readSource: DispatchSourceRead?

    init(fileDescriptor: Int32) {
        self.fileDescriptor = fileDescriptor
    }
}

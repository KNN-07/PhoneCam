import AVFoundation
import Foundation
import VideoToolbox

final class VideoEncoder {
    enum EncoderError: Error {
        case sessionCreationFailed(OSStatus)
        case propertyFailed(CFString, OSStatus)
        case prepareFailed(OSStatus)
        case encodeFailed(OSStatus)
        case hardwareEncoderUnavailable
        case malformedAccessUnit
        case missingParameterSets
    }

    var onEncodedNALUnit: ((Data, UInt64, Bool) -> Void)?
    var onFatalError: ((EncoderError) -> Void)?

    private static let startCode = Data([0, 0, 0, 1])
    private let encoderQueue = DispatchQueue(label: "com.phonecam.ios.video-encoder")
    private var compressionSession: VTCompressionSession?
    private var forceNextKeyFrame = false

    let codec: VideoCodec
    private let width: Int32
    private let height: Int32
    private let fps: Int32
    private let bitrate: Int

    init(codec: VideoCodec, width: Int32, height: Int32, fps: Int32, bitrate: Int) {
        self.codec = codec
        self.width = width
        self.height = height
        self.fps = fps
        self.bitrate = bitrate
    }

    deinit { stop() }

    func start() throws {
        try encoderQueue.sync {
            guard compressionSession == nil else { return }
            try createCompressionSession()
        }
    }

    func stop() {
        encoderQueue.sync {
            guard let session = compressionSession else { return }
            VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(session)
            compressionSession = nil
            forceNextKeyFrame = false
        }
    }

    func requestKeyFrame() {
        encoderQueue.async { self.forceNextKeyFrame = true }
    }

    func encode(sampleBuffer: CMSampleBuffer) {
        encoderQueue.async { [weak self] in
            guard let self, let session = self.compressionSession,
                  let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer)
            else { return }
            let dimensions = CVImageBufferGetEncodedSize(imageBuffer)
            guard Int32(dimensions.width) == self.width, Int32(dimensions.height) == self.height else {
                self.onFatalError?(.malformedAccessUnit)
                return
            }
            var properties: CFDictionary?
            if self.forceNextKeyFrame {
                self.forceNextKeyFrame = false
                properties = [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary
            }
            var flags = VTEncodeInfoFlags()
            let status = VTCompressionSessionEncodeFrame(
                session,
                imageBuffer: imageBuffer,
                presentationTimeStamp: CMSampleBufferGetPresentationTimeStamp(sampleBuffer),
                duration: CMSampleBufferGetDuration(sampleBuffer),
                frameProperties: properties,
                sourceFrameRefcon: nil,
                infoFlagsOut: &flags
            )
            if status != noErr { self.onFatalError?(.encodeFailed(status)) }
        }
    }

    private func createCompressionSession() throws {
        var session: VTCompressionSession?
        let specification: CFDictionary?
        if codec == .hevc {
            guard #available(iOS 17.4, *) else {
                throw EncoderError.hardwareEncoderUnavailable
            }
            specification = [
                kVTVideoEncoderSpecification_RequireHardwareAcceleratedVideoEncoder: true
            ] as CFDictionary
        } else {
            specification = nil
        }
        let status = VTCompressionSessionCreate(
            allocator: kCFAllocatorDefault,
            width: width,
            height: height,
            codecType: codec.cmCodecType,
            encoderSpecification: specification,
            imageBufferAttributes: nil,
            compressedDataAllocator: nil,
            outputCallback: compressionOutputCallback,
            refcon: Unmanaged.passUnretained(self).toOpaque(),
            compressionSessionOut: &session
        )
        guard status == noErr, let session else { throw EncoderError.sessionCreationFailed(status) }
        compressionSession = session

        try set(session, kVTCompressionPropertyKey_RealTime, kCFBooleanTrue)
        try set(session, kVTCompressionPropertyKey_ProfileLevel, codec.profileLevel)
        try set(session, kVTCompressionPropertyKey_AllowFrameReordering, kCFBooleanFalse)
        try set(session, kVTCompressionPropertyKey_ExpectedFrameRate, NSNumber(value: fps))
        try set(session, kVTCompressionPropertyKey_MaxKeyFrameInterval, NSNumber(value: fps))
        try set(session, kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, NSNumber(value: 1))
        try set(session, kVTCompressionPropertyKey_AverageBitRate, NSNumber(value: bitrate))
        let bytesPerSecond = Int64(bitrate) * 3 / 2 / 8
        try set(
            session,
            kVTCompressionPropertyKey_DataRateLimits,
            [NSNumber(value: bytesPerSecond), NSNumber(value: 1)] as CFArray
        )
        let prepareStatus = VTCompressionSessionPrepareToEncodeFrames(session)
        guard prepareStatus == noErr else { throw EncoderError.prepareFailed(prepareStatus) }
        if codec == .hevc {
            guard #available(iOS 17.4, *) else {
                throw EncoderError.hardwareEncoderUnavailable
            }
            var value: CFTypeRef?
            let hardwareStatus = VTSessionCopyProperty(
                session,
                key: kVTCompressionPropertyKey_UsingHardwareAcceleratedVideoEncoder,
                allocator: kCFAllocatorDefault,
                valueOut: &value
            )
            guard hardwareStatus == noErr, (value as? Bool) == true else {
                throw EncoderError.hardwareEncoderUnavailable
            }
        }
    }

    private func set(_ session: VTCompressionSession, _ key: CFString, _ value: CFTypeRef) throws {
        let status = VTSessionSetProperty(session, key: key, value: value)
        guard status == noErr else { throw EncoderError.propertyFailed(key, status) }
    }

    fileprivate func handleCompressedSampleBuffer(_ sampleBuffer: CMSampleBuffer) {
        guard CMSampleBufferDataIsReady(sampleBuffer),
              let format = CMSampleBufferGetFormatDescription(sampleBuffer),
              let block = CMSampleBufferGetDataBuffer(sampleBuffer)
        else {
            onFatalError?(.malformedAccessUnit)
            return
        }
        let keyframe = Self.isKeyframe(sampleBuffer)
        do {
            let parameterInfo = try parameterSets(from: format, include: keyframe)
            let length = CMBlockBufferGetDataLength(block)
            var bytes = Data(count: length)
            let copyStatus = bytes.withUnsafeMutableBytes { raw in
                CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: length, destination: raw.baseAddress!)
            }
            guard copyStatus == kCMBlockBufferNoErr else { throw EncoderError.malformedAccessUnit }
            let payload = try Self.makeAnnexB(
                lengthPrefixedData: bytes,
                nalLengthBytes: parameterInfo.headerLength,
                parameterSets: parameterInfo.sets
            )
            onEncodedNALUnit?(
                payload,
                Self.toMicroseconds(CMSampleBufferGetPresentationTimeStamp(sampleBuffer)),
                keyframe
            )
        } catch let error as EncoderError {
            onFatalError?(error)
        } catch {
            onFatalError?(.malformedAccessUnit)
        }
    }

    private func parameterSets(
        from format: CMFormatDescription,
        include: Bool
    ) throws -> (sets: [Data], headerLength: Int) {
        var count = 0
        var headerLength: Int32 = 0
        let status: OSStatus
        switch codec {
        case .h264:
            status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                parameterSetSizeOut: nil, parameterSetCountOut: &count,
                nalUnitHeaderLengthOut: &headerLength
            )
        case .hevc:
            status = CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
                format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                parameterSetSizeOut: nil, parameterSetCountOut: &count,
                nalUnitHeaderLengthOut: &headerLength
            )
        }
        guard status == noErr, (1...4).contains(Int(headerLength)) else {
            throw EncoderError.malformedAccessUnit
        }
        guard include else { return ([], Int(headerLength)) }
        guard count > 0 else { throw EncoderError.missingParameterSets }
        var sets: [Data] = []
        for index in 0..<count {
            var pointer: UnsafePointer<UInt8>?
            var size = 0
            let itemStatus: OSStatus
            switch codec {
            case .h264:
                itemStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                    format, parameterSetIndex: index, parameterSetPointerOut: &pointer,
                    parameterSetSizeOut: &size, parameterSetCountOut: nil,
                    nalUnitHeaderLengthOut: nil
                )
            case .hevc:
                itemStatus = CMVideoFormatDescriptionGetHEVCParameterSetAtIndex(
                    format, parameterSetIndex: index, parameterSetPointerOut: &pointer,
                    parameterSetSizeOut: &size, parameterSetCountOut: nil,
                    nalUnitHeaderLengthOut: nil
                )
            }
            guard itemStatus == noErr, let pointer, size > 0 else {
                throw EncoderError.missingParameterSets
            }
            sets.append(Data(bytes: pointer, count: size))
        }
        return (sets, Int(headerLength))
    }

    static func makeAnnexB(
        lengthPrefixedData: Data,
        nalLengthBytes: Int,
        parameterSets: [Data]
    ) throws -> Data {
        guard (1...4).contains(nalLengthBytes) else { throw EncoderError.malformedAccessUnit }
        var output = Data()
        for set in parameterSets {
            output.append(startCode)
            output.append(set)
        }
        var offset = 0
        while offset < lengthPrefixedData.count {
            guard offset + nalLengthBytes <= lengthPrefixedData.count else {
                throw EncoderError.malformedAccessUnit
            }
            var length = 0
            for byte in lengthPrefixedData[offset..<(offset + nalLengthBytes)] {
                length = (length << 8) | Int(byte)
            }
            offset += nalLengthBytes
            guard length > 0, offset + length <= lengthPrefixedData.count else {
                throw EncoderError.malformedAccessUnit
            }
            output.append(startCode)
            output.append(lengthPrefixedData[offset..<(offset + length)])
            offset += length
        }
        guard !output.isEmpty else { throw EncoderError.malformedAccessUnit }
        return output
    }

    private static func isKeyframe(_ sampleBuffer: CMSampleBuffer) -> Bool {
        guard let attachments = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false),
              CFArrayGetCount(attachments) > 0,
              let dictionary = unsafeBitCast(CFArrayGetValueAtIndex(attachments, 0), to: CFDictionary?.self)
                as? [CFString: Any]
        else { return true }
        return !(dictionary[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
    }

    private static func toMicroseconds(_ time: CMTime) -> UInt64 {
        guard time.timescale > 0 else { return 0 }
        return UInt64(max(0, time.value * 1_000_000 / Int64(time.timescale)))
    }
}

private let compressionOutputCallback: VTCompressionOutputCallback = {
    refcon, _, status, _, sampleBuffer in
    guard let refcon else { return }
    let encoder = Unmanaged<VideoEncoder>.fromOpaque(refcon).takeUnretainedValue()
    guard status == noErr, let sampleBuffer else {
        encoder.onFatalError?(.encodeFailed(status))
        return
    }
    encoder.handleCompressedSampleBuffer(sampleBuffer)
}

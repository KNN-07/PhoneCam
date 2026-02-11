import AVFoundation
import Foundation
import VideoToolbox

final class H264Encoder {
    enum EncoderError: Error {
        case sessionCreationFailed(OSStatus)
        case sessionUnavailable
    }

    var onEncodedNALUnit: ((Data, UInt64, Bool) -> Void)?

    private static let naluStartCode: [UInt8] = [0x00, 0x00, 0x00, 0x01]

    private let encoderQueue = DispatchQueue(label: "com.phonecam.ios.h264-encoder")
    private var compressionSession: VTCompressionSession?
    private var forceNextKeyFrame = false

    private var width: Int32
    private var height: Int32
    private var fps: Int32
    private var bitrate: Int

    init(width: Int32, height: Int32, fps: Int32 = 30, bitrate: Int = 4_000_000) {
        self.width = width
        self.height = height
        self.fps = fps
        self.bitrate = bitrate
    }

    deinit {
        stop()
    }

    func start() throws {
        try encoderQueue.sync {
            if compressionSession == nil {
                try createCompressionSession()
            }
        }
    }

    func stop() {
        encoderQueue.sync {
            guard let compressionSession else {
                return
            }

            VTCompressionSessionCompleteFrames(compressionSession, untilPresentationTimeStamp: .invalid)
            VTCompressionSessionInvalidate(compressionSession)
            self.compressionSession = nil
            self.forceNextKeyFrame = false
        }
    }

    func requestKeyFrame() {
        encoderQueue.async {
            self.forceNextKeyFrame = true
        }
    }

    func updateConfiguration(width: Int32, height: Int32, fps: Int32, bitrate: Int) throws {
        try encoderQueue.sync {
            self.width = width
            self.height = height
            self.fps = fps
            self.bitrate = bitrate

            if let session = compressionSession {
                VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
                VTCompressionSessionInvalidate(session)
                compressionSession = nil
            }

            try createCompressionSession()
            forceNextKeyFrame = true
        }
    }

    func encode(sampleBuffer: CMSampleBuffer) {
        encoderQueue.async { [weak self] in
            guard let self else {
                return
            }

            guard let compressionSession = self.compressionSession else {
                return
            }

            guard let imageBuffer = CMSampleBufferGetImageBuffer(sampleBuffer) else {
                return
            }

            let presentationTimeStamp = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
            let duration = CMSampleBufferGetDuration(sampleBuffer)
            var infoFlags = VTEncodeInfoFlags()
            var frameProperties: CFDictionary?

            if self.forceNextKeyFrame {
                self.forceNextKeyFrame = false
                let options: [CFString: Any] = [kVTEncodeFrameOptionKey_ForceKeyFrame: true]
                frameProperties = options as CFDictionary
            }

            VTCompressionSessionEncodeFrame(
                compressionSession,
                imageBuffer: imageBuffer,
                presentationTimeStamp: presentationTimeStamp,
                duration: duration,
                frameProperties: frameProperties,
                sourceFrameRefcon: nil,
                infoFlagsOut: &infoFlags
            )
        }
    }

    private func createCompressionSession() throws {
        var newSession: VTCompressionSession?

        let status = VTCompressionSessionCreate(
            allocator: kCFAllocatorDefault,
            width: width,
            height: height,
            codecType: kCMVideoCodecType_H264,
            encoderSpecification: nil,
            imageBufferAttributes: nil,
            compressedDataAllocator: nil,
            outputCallback: compressionOutputCallback,
            refcon: Unmanaged.passUnretained(self).toOpaque(),
            compressionSessionOut: &newSession
        )

        guard status == noErr, let newSession else {
            throw EncoderError.sessionCreationFailed(status)
        }

        compressionSession = newSession

        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_RealTime, value: kCFBooleanTrue)
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_ProfileLevel, value: kVTProfileLevel_H264_Baseline_AutoLevel)
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_AllowFrameReordering, value: kCFBooleanFalse)

        let frameRate = NSNumber(value: fps)
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_ExpectedFrameRate, value: frameRate)
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_MaxKeyFrameInterval, value: frameRate)
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration, value: NSNumber(value: 1))

        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_AverageBitRate, value: NSNumber(value: bitrate))
        let dataRateLimits: [NSNumber] = [NSNumber(value: bitrate / 8), NSNumber(value: 1)]
        VTSessionSetProperty(newSession, key: kVTCompressionPropertyKey_DataRateLimits, value: dataRateLimits as CFArray)

        VTCompressionSessionPrepareToEncodeFrames(newSession)
    }

    private func handleCompressedSampleBuffer(_ sampleBuffer: CMSampleBuffer) {
        guard CMSampleBufferDataIsReady(sampleBuffer) else {
            return
        }

        let isKeyframe = Self.isKeyframe(sampleBuffer)
        guard let payload = buildAnnexBPayload(from: sampleBuffer, includeParameterSets: isKeyframe) else {
            return
        }

        let pts = CMSampleBufferGetPresentationTimeStamp(sampleBuffer)
        onEncodedNALUnit?(payload, Self.toMicroseconds(pts), isKeyframe)
    }

    private func buildAnnexBPayload(from sampleBuffer: CMSampleBuffer, includeParameterSets: Bool) -> Data? {
        var payload = Data()

        if includeParameterSets {
            appendParameterSets(from: sampleBuffer, into: &payload)
        }

        guard let blockBuffer = CMSampleBufferGetDataBuffer(sampleBuffer) else {
            return nil
        }

        var totalLength = 0
        var dataPointer: UnsafeMutablePointer<Int8>?
        let status = CMBlockBufferGetDataPointer(
            blockBuffer,
            atOffset: 0,
            lengthAtOffsetOut: nil,
            totalLengthOut: &totalLength,
            dataPointerOut: &dataPointer
        )

        guard status == noErr, let dataPointer else {
            return nil
        }

        var offset = 0
        let avccHeaderLength = 4

        while offset + avccHeaderLength <= totalLength {
            var naluLengthBE: UInt32 = 0
            memcpy(&naluLengthBE, dataPointer.advanced(by: offset), avccHeaderLength)
            let naluLength = Int(CFSwapInt32BigToHost(naluLengthBE))

            offset += avccHeaderLength
            guard naluLength > 0, offset + naluLength <= totalLength else {
                break
            }

            payload.append(contentsOf: Self.naluStartCode)

            let naluPointer = UnsafeRawPointer(dataPointer.advanced(by: offset)).assumingMemoryBound(to: UInt8.self)
            payload.append(naluPointer, count: naluLength)

            offset += naluLength
        }

        return payload.isEmpty ? nil : payload
    }

    private func appendParameterSets(from sampleBuffer: CMSampleBuffer, into payload: inout Data) {
        guard let formatDescription = CMSampleBufferGetFormatDescription(sampleBuffer) else {
            return
        }

        var parameterSetCount: Int = 0
        var nalUnitHeaderLength: Int32 = 0

        let countStatus = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
            formatDescription,
            parameterSetIndex: 0,
            parameterSetPointerOut: nil,
            parameterSetSizeOut: nil,
            parameterSetCountOut: &parameterSetCount,
            nalUnitHeaderLengthOut: &nalUnitHeaderLength
        )

        guard countStatus == noErr, parameterSetCount > 0 else {
            return
        }

        for index in 0 ..< parameterSetCount {
            var parameterSetPointer: UnsafePointer<UInt8>?
            var parameterSetSize: Int = 0

            let status = CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                formatDescription,
                parameterSetIndex: index,
                parameterSetPointerOut: &parameterSetPointer,
                parameterSetSizeOut: &parameterSetSize,
                parameterSetCountOut: nil,
                nalUnitHeaderLengthOut: nil
            )

            guard status == noErr, let parameterSetPointer, parameterSetSize > 0 else {
                continue
            }

            payload.append(contentsOf: Self.naluStartCode)
            payload.append(parameterSetPointer, count: parameterSetSize)
        }
    }

    private static func isKeyframe(_ sampleBuffer: CMSampleBuffer) -> Bool {
        guard
            let attachmentArray = CMSampleBufferGetSampleAttachmentsArray(sampleBuffer, createIfNecessary: false),
            CFArrayGetCount(attachmentArray) > 0,
            let attachments = unsafeBitCast(CFArrayGetValueAtIndex(attachmentArray, 0), to: CFDictionary?.self) as? [CFString: Any]
        else {
            return true
        }

        if let notSync = attachments[kCMSampleAttachmentKey_NotSync] as? Bool {
            return !notSync
        }

        return true
    }

    private static func toMicroseconds(_ time: CMTime) -> UInt64 {
        guard time.timescale > 0 else {
            return 0
        }

        let value = (time.value * 1_000_000) / Int64(time.timescale)
        return UInt64(max(0, value))
    }
}

private let compressionOutputCallback: VTCompressionOutputCallback = {
    outputCallbackRefCon,
    _,
    status,
    _,
    sampleBuffer
    in
    guard status == noErr, let sampleBuffer, let outputCallbackRefCon else {
        return
    }

    let encoder = Unmanaged<H264Encoder>.fromOpaque(outputCallbackRefCon).takeUnretainedValue()
    encoder.handleCompressedSampleBuffer(sampleBuffer)
}

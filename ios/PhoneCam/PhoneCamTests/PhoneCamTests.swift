import XCTest
@testable import PhoneCam

final class PhoneCamTests: XCTestCase {
    func testResolutionDimensionsAndBitrates() {
        let expected: [(CaptureResolution, Int32, Int32, Int)] = [
            (.p480, 640, 480, 2_000_000),
            (.p720, 1280, 720, 4_000_000),
            (.p1080, 1920, 1080, 8_000_000),
            (.p1440, 2560, 1440, 16_000_000),
            (.p2160, 3840, 2160, 35_000_000),
        ]
        XCTAssertEqual(CaptureResolution.allCases.count, expected.count)
        for (resolution, width, height, bitrate) in expected {
            XCTAssertEqual(resolution.dimensions.width, width)
            XCTAssertEqual(resolution.dimensions.height, height)
            XCTAssertEqual(resolution.targetBitrate, bitrate)
            XCTAssertEqual(
                CaptureResolution.matching(width: UInt16(width), height: UInt16(height)),
                resolution
            )
        }
    }

    func testCodecResolutionAndFrameRateBitrateTable() {
        let dimensions: [(UInt16, UInt16)] = [
            (640, 480), (1280, 720), (1920, 1080), (2560, 1440), (3840, 2160),
        ]
        let h264 = [
            [1_000_000, 2_000_000, 3_000_000],
            [2_500_000, 4_000_000, 7_000_000],
            [4_000_000, 8_000_000, 12_000_000],
            [8_000_000, 16_000_000, 24_000_000],
            [16_000_000, 35_000_000, 50_000_000],
        ]
        let hevc = [
            [750_000, 1_250_000, 2_000_000],
            [1_500_000, 2_500_000, 4_500_000],
            [2_500_000, 5_000_000, 8_000_000],
            [5_000_000, 9_000_000, 15_000_000],
            [10_000_000, 20_000_000, 32_000_000],
        ]
        for (resolutionIndex, size) in dimensions.enumerated() {
            for (rateIndex, fps) in [15, 30, 60].enumerated() {
                XCTAssertEqual(
                    StreamManager.bitrate(
                        for: StreamProfile(
                            codec: .h264,
                            width: size.0,
                            height: size.1,
                            fps: UInt8(fps)
                        )
                    ),
                    h264[resolutionIndex][rateIndex]
                )
                XCTAssertEqual(
                    StreamManager.bitrate(
                        for: StreamProfile(
                            codec: .hevc,
                            width: size.0,
                            height: size.1,
                            fps: UInt8(fps)
                        )
                    ),
                    hevc[resolutionIndex][rateIndex]
                )
            }
        }
    }

    func testCaptureFormatRankingIsExactAndDeterministic() {
        let descriptors = [
            CaptureFormatDescriptor(index: 0, width: 1920, height: 1080, minimumFrameRate: 24, maximumFrameRate: 60, isFullRangeNV12: false, isBinned: false),
            CaptureFormatDescriptor(index: 1, width: 1920, height: 1080, minimumFrameRate: 30, maximumFrameRate: 60, isFullRangeNV12: true, isBinned: true),
            CaptureFormatDescriptor(index: 2, width: 1920, height: 1080, minimumFrameRate: 30, maximumFrameRate: 60, isFullRangeNV12: true, isBinned: false),
            CaptureFormatDescriptor(index: 3, width: 3840, height: 2160, minimumFrameRate: 30, maximumFrameRate: 30, isFullRangeNV12: true, isBinned: false),
        ]
        XCTAssertEqual(
            selectCaptureFormat(from: descriptors, width: 1920, height: 1080, fps: 60)?.index,
            2
        )
        XCTAssertNil(selectCaptureFormat(from: descriptors, width: 3840, height: 2160, fps: 60))
        XCTAssertNil(selectCaptureFormat(from: descriptors, width: 1280, height: 720, fps: 30))
    }

    func testHEVCTransactionFallsBackOnlyToH264AtTheSameTuple() {
        let requested = StreamProfile(codec: .hevc, width: 3840, height: 2160, fps: 60)
        let fallback = StreamProfile(codec: .h264, width: 3840, height: 2160, fps: 60)
        XCTAssertEqual(
            streamConfigurationCandidates(
                requested: requested,
                available: [requested, fallback]
            ),
            [requested, fallback]
        )
        XCTAssertEqual(
            streamConfigurationCandidates(requested: requested, available: [requested]),
            [requested]
        )
        XCTAssertEqual(
            streamConfigurationCandidates(requested: fallback, available: [requested, fallback]),
            [fallback]
        )
    }

    func testLengthPrefixedAccessUnitsConvertForAllHeaderWidths() throws {
        for headerWidth in 1...4 {
            let nal = Data([0x65, 0xAA, 0xBB])
            var packet = Data(repeating: 0, count: headerWidth)
            packet[headerWidth - 1] = UInt8(nal.count)
            packet.append(nal)
            XCTAssertEqual(
                try VideoEncoder.makeAnnexB(
                    lengthPrefixedData: packet,
                    nalLengthBytes: headerWidth,
                    parameterSets: []
                ),
                Data([0, 0, 0, 1, 0x65, 0xAA, 0xBB])
            )
        }
    }

    func testParameterSetsPrecedeAccessUnitInSuppliedCodecOrder() throws {
        let payload = try VideoEncoder.makeAnnexB(
            lengthPrefixedData: Data([0, 0, 0, 2, 0x26, 0x01]),
            nalLengthBytes: 4,
            parameterSets: [Data([0x40]), Data([0x42]), Data([0x44])]
        )
        XCTAssertEqual(
            payload,
            Data([
                0, 0, 0, 1, 0x40,
                0, 0, 0, 1, 0x42,
                0, 0, 0, 1, 0x44,
                0, 0, 0, 1, 0x26, 0x01,
            ])
        )
    }

    func testMalformedLengthPrefixesAreRejected() {
        XCTAssertThrowsError(
            try VideoEncoder.makeAnnexB(
                lengthPrefixedData: Data([0, 0, 0, 4, 0x65]),
                nalLengthBytes: 4,
                parameterSets: []
            )
        )
        XCTAssertThrowsError(
            try VideoEncoder.makeAnnexB(
                lengthPrefixedData: Data([1, 0x65]),
                nalLengthBytes: 0,
                parameterSets: []
            )
        )
    }
}

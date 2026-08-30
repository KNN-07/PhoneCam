package com.phonecam.app

import org.junit.Assert.assertArrayEquals
import org.junit.Test

class EncodedNalUnitTest {
    @Test
    fun preservesAnnexBPayload() {
        val payload = byteArrayOf(0, 0, 0, 1, 0x65, 1, 2, 3)
        assertArrayEquals(payload, EncodedNalUnit.toAnnexB(payload, VideoCodec.H264))
    }

    @Test
    fun convertsAvccLengthPrefixes() {
        val payload = byteArrayOf(0, 0, 0, 2, 0x67, 1, 0, 0, 0, 3, 0x65, 2, 3)
        val expected = byteArrayOf(0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 2, 3)
        assertArrayEquals(expected, EncodedNalUnit.toAnnexB(payload, VideoCodec.H264))
    }

    @Test
    fun convertsHvccLengthPrefixesWithVpsSpsPpsAndIdr() {
        val payload =
            byteArrayOf(
                0, 0, 0, 2, 0x40, 1,
                0, 0, 0, 2, 0x42, 1,
                0, 0, 0, 2, 0x44, 1,
                0, 0, 0, 2, 0x26, 1,
            )
        val expected =
            byteArrayOf(
                0, 0, 0, 1, 0x40, 1,
                0, 0, 0, 1, 0x42, 1,
                0, 0, 0, 1, 0x44, 1,
                0, 0, 0, 1, 0x26, 1,
            )
        assertArrayEquals(expected, EncodedNalUnit.toAnnexB(payload, VideoCodec.HEVC))
    }

    @Test
    fun leavesMalformedLengthPrefixUnchanged() {
        val payload = byteArrayOf(0, 0, 0, 9, 0x65, 1)
        assertArrayEquals(payload, EncodedNalUnit.toAnnexB(payload, VideoCodec.H264))
    }

    @Test
    fun prefixesCodecParameterSetsToEveryKeyframe() {
        val parameterSets = byteArrayOf(0, 0, 0, 1, 0x40, 1, 0, 0, 0, 1, 0x42, 1)
        val idr = byteArrayOf(0, 0, 0, 1, 0x26, 1)
        assertArrayEquals(
            parameterSets + idr,
            EncodedNalUnit.prefixParameterSets(parameterSets, idr, true),
        )
        assertArrayEquals(idr, EncodedNalUnit.prefixParameterSets(parameterSets, idr, false))
    }
}

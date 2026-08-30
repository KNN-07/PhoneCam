package com.phonecam.app

import org.junit.Assert.assertArrayEquals
import org.junit.Test

class H264NalUnitTest {
    @Test
    fun preservesAnnexBPayload() {
        val payload = byteArrayOf(0, 0, 0, 1, 0x65, 1, 2, 3)
        assertArrayEquals(payload, H264NalUnit.toAnnexB(payload))
    }

    @Test
    fun convertsAvccLengthPrefixes() {
        val payload = byteArrayOf(0, 0, 0, 2, 0x67, 1, 0, 0, 0, 3, 0x65, 2, 3)
        val expected = byteArrayOf(0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x65, 2, 3)
        assertArrayEquals(expected, H264NalUnit.toAnnexB(payload))
    }
}

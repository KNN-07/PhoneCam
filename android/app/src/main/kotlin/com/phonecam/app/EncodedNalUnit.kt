package com.phonecam.app

internal object EncodedNalUnit {
    private val startCode = byteArrayOf(0, 0, 0, 1)

    fun toAnnexB(payload: ByteArray, codec: VideoCodec): ByteArray {
        if (payload.isEmpty() || hasStartCode(payload)) {
            return payload
        }

        val converted = convertLengthPrefixed(payload, codec)
        if (converted != null) {
            return converted
        }

        if (isValidHeader(payload[0], codec)) {
            return startCode + payload
        }

        return payload
    }
    fun prefixParameterSets(
        codecConfig: ByteArray?,
        accessUnit: ByteArray,
        isKeyframe: Boolean,
    ): ByteArray {
        if (!isKeyframe) {
            return accessUnit
        }
        val config = codecConfig
        require(config != null && config.isNotEmpty()) {
            "Encoder emitted an IDR before codec parameter sets"
        }
        return ByteArray(config.size + accessUnit.size).also {
            config.copyInto(it)
            accessUnit.copyInto(it, config.size)
        }
    }


    private fun hasStartCode(payload: ByteArray): Boolean =
        payload.size >= 3 &&
            payload[0] == 0.toByte() &&
            payload[1] == 0.toByte() &&
            (payload[2] == 1.toByte() || (payload.size >= 4 && payload[2] == 0.toByte() && payload[3] == 1.toByte()))

    private fun convertLengthPrefixed(payload: ByteArray, codec: VideoCodec): ByteArray? {
        var offset = 0
        while (offset < payload.size) {
            if (payload.size - offset < 4) {
                return null
            }
            val length = readLength(payload, offset)
            offset += 4
            if (length <= 0 || length > payload.size - offset || !isValidHeader(payload[offset], codec)) {
                return null
            }
            offset += length
        }

        val output = ByteArray(payload.size)
        offset = 0
        var outputOffset = 0
        while (offset < payload.size) {
            val length = readLength(payload, offset)
            offset += 4
            startCode.copyInto(output, outputOffset)
            outputOffset += startCode.size
            payload.copyInto(output, outputOffset, offset, offset + length)
            outputOffset += length
            offset += length
        }
        return output
    }

    private fun readLength(payload: ByteArray, offset: Int): Int =
        ((payload[offset].toInt() and 0xff) shl 24) or
            ((payload[offset + 1].toInt() and 0xff) shl 16) or
            ((payload[offset + 2].toInt() and 0xff) shl 8) or
            (payload[offset + 3].toInt() and 0xff)

    private fun isValidHeader(
        header: Byte,
        codec: VideoCodec,
    ): Boolean {
        val unsigned = header.toInt() and 0xff
        return when (codec) {
            VideoCodec.H264 -> unsigned and 0x80 == 0 && (unsigned and 0x1f) in 1..23
            VideoCodec.HEVC -> unsigned and 0x80 == 0 && ((unsigned shr 1) and 0x3f) in 0..47
        }
    }
}

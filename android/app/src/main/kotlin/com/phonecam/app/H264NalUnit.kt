package com.phonecam.app

internal object H264NalUnit {
    private val startCode = byteArrayOf(0, 0, 0, 1)

    fun toAnnexB(payload: ByteArray): ByteArray {
        if (payload.isEmpty() || hasStartCode(payload)) {
            return payload
        }

        val converted = convertLengthPrefixed(payload)
        if (converted != null) {
            return converted
        }

        if (isValidHeader(payload[0])) {
            return startCode + payload
        }

        return payload
    }

    private fun hasStartCode(payload: ByteArray): Boolean =
        payload.size >= 3 &&
            payload[0] == 0.toByte() &&
            payload[1] == 0.toByte() &&
            (payload[2] == 1.toByte() || (payload.size >= 4 && payload[2] == 0.toByte() && payload[3] == 1.toByte()))

    private fun convertLengthPrefixed(payload: ByteArray): ByteArray? {
        val output = ArrayList<Byte>(payload.size + 4)
        var offset = 0

        while (offset < payload.size) {
            if (payload.size - offset < 4) {
                return null
            }

            val length =
                ((payload[offset].toInt() and 0xff) shl 24) or
                    ((payload[offset + 1].toInt() and 0xff) shl 16) or
                    ((payload[offset + 2].toInt() and 0xff) shl 8) or
                    (payload[offset + 3].toInt() and 0xff)
            offset += 4

            if (length <= 0 || length > payload.size - offset || !isValidHeader(payload[offset])) {
                return null
            }

            output.addAll(startCode.toList())
            for (index in offset until offset + length) {
                output.add(payload[index])
            }
            offset += length
        }

        return output.toByteArray().takeIf { it.isNotEmpty() }
    }

    private fun isValidHeader(header: Byte): Boolean {
        val unsigned = header.toInt() and 0xff
        val nalType = unsigned and 0x1f
        return unsigned and 0x80 == 0 && nalType in 1..23
    }
}

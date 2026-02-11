package com.phonecam.app

import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.os.Bundle
import android.util.Log
import androidx.camera.core.ImageProxy

class H264Encoder(
    private val width: Int = 1280,
    private val height: Int = 720,
    private val bitRate: Int = 4_000_000,
    private val frameRate: Int = 30,
    private val iFrameIntervalSec: Int = 2,
    private val onNalUnitReady: (nalUnit: ByteArray, ptsUs: Long, isKeyframe: Boolean) -> Unit,
) {
    private var mediaCodec: MediaCodec? = null
    private var colorFormat: Int = MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible
    private val bufferInfo = MediaCodec.BufferInfo()
    private var started = false

    fun start() {
        if (started) {
            return
        }

        val codec = MediaCodec.createEncoderByType(MIME_TYPE)
        val capabilities = codec.codecInfo.getCapabilitiesForType(MIME_TYPE)
        colorFormat = selectColorFormat(capabilities.colorFormats)

        val format =
            MediaFormat.createVideoFormat(MIME_TYPE, width, height).apply {
                setInteger(MediaFormat.KEY_COLOR_FORMAT, colorFormat)
                setInteger(MediaFormat.KEY_BIT_RATE, bitRate)
                setInteger(MediaFormat.KEY_FRAME_RATE, frameRate)
                setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, iFrameIntervalSec)

                runCatching {
                    setInteger(
                        MediaFormat.KEY_BITRATE_MODE,
                        MediaCodecInfo.EncoderCapabilities.BITRATE_MODE_CBR,
                    )
                }
                runCatching {
                    setInteger(
                        MediaFormat.KEY_PROFILE,
                        MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline,
                    )
                }
                runCatching {
                    setInteger(
                        MediaFormat.KEY_LEVEL,
                        MediaCodecInfo.CodecProfileLevel.AVCLevel31,
                    )
                }
            }

        codec.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        codec.start()

        mediaCodec = codec
        started = true

        Log.i(TAG, "H.264 encoder started at ${width}x$height @ ${bitRate}bps")
    }

    fun encode(imageProxy: ImageProxy) {
        val codec = mediaCodec ?: return
        if (!started) {
            return
        }

        if (imageProxy.width != width || imageProxy.height != height) {
            Log.w(
                TAG,
                "Dropping frame due to resolution mismatch. got=${imageProxy.width}x${imageProxy.height}, expected=${width}x$height",
            )
            return
        }

        val frameData = convertImageToEncoderFormat(imageProxy, colorFormat)
        val inputBufferIndex = codec.dequeueInputBuffer(0)
        if (inputBufferIndex < 0) {
            drainOutput(codec)
            return
        }

        val inputBuffer = codec.getInputBuffer(inputBufferIndex)
        if (inputBuffer == null) {
            drainOutput(codec)
            return
        }

        if (frameData.size > inputBuffer.capacity()) {
            Log.w(TAG, "Dropping frame larger than codec input buffer capacity")
            drainOutput(codec)
            return
        }

        inputBuffer.clear()
        inputBuffer.put(frameData)
        codec.queueInputBuffer(
            inputBufferIndex,
            0,
            frameData.size,
            imageProxy.imageInfo.timestamp / 1_000,
            0,
        )

        drainOutput(codec)
    }

    fun stop() {
        val codec = mediaCodec ?: return

        runCatching {
            drainOutput(codec)
            codec.stop()
        }.onFailure {
            Log.w(TAG, "Failed to stop H.264 encoder cleanly", it)
        }

        runCatching {
            codec.release()
        }.onFailure {
            Log.w(TAG, "Failed to release H.264 encoder", it)
        }

        mediaCodec = null
        started = false
    }

    fun requestKeyFrame() {
        val codec = mediaCodec ?: return
        if (!started) {
            return
        }

        runCatching {
            codec.setParameters(
                Bundle().apply {
                    putInt(MediaCodec.PARAMETER_KEY_REQUEST_SYNC_FRAME, 0)
                },
            )
        }.onFailure {
            Log.w(TAG, "Failed to request keyframe", it)
        }
    }

    private fun drainOutput(codec: MediaCodec) {
        while (true) {
            val outputBufferIndex = codec.dequeueOutputBuffer(bufferInfo, 0)
            when {
                outputBufferIndex >= 0 -> {
                    val outputBuffer = codec.getOutputBuffer(outputBufferIndex)
                    if (outputBuffer != null && bufferInfo.size > 0) {
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size)

                        val nalUnit = ByteArray(bufferInfo.size)
                        outputBuffer.get(nalUnit)

                        val isCodecConfig =
                            (bufferInfo.flags and MediaCodec.BUFFER_FLAG_CODEC_CONFIG) != 0
                        val isKeyframe =
                            isCodecConfig ||
                                (bufferInfo.flags and MediaCodec.BUFFER_FLAG_KEY_FRAME) != 0

                        onNalUnitReady(
                            nalUnit,
                            bufferInfo.presentationTimeUs,
                            isKeyframe,
                        )
                    }
                    codec.releaseOutputBuffer(outputBufferIndex, false)
                }

                outputBufferIndex == MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> {
                    Log.i(TAG, "Encoder output format changed: ${codec.outputFormat}")
                }

                outputBufferIndex == MediaCodec.INFO_TRY_AGAIN_LATER -> {
                    return
                }

                else -> {
                    return
                }
            }
        }
    }

    private fun selectColorFormat(colorFormats: IntArray): Int {
        val preferredColorFormats =
            listOf(
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420PackedSemiPlanar,
                MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420PackedPlanar,
                COLOR_TI_FORMAT_YUV420_PACKED_SEMIPLANAR,
            )

        return preferredColorFormats.firstOrNull { it in colorFormats }
            ?: throw IllegalStateException(
                "No supported YUV420 color format for H.264 encoder",
            )
    }

    private fun convertImageToEncoderFormat(
        imageProxy: ImageProxy,
        targetColorFormat: Int,
    ): ByteArray {
        val i420 = yuv420888ToI420(imageProxy)
        return when (targetColorFormat) {
            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Planar,
            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420PackedPlanar,
            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420Flexible,
            -> i420

            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420SemiPlanar,
            MediaCodecInfo.CodecCapabilities.COLOR_FormatYUV420PackedSemiPlanar,
            COLOR_TI_FORMAT_YUV420_PACKED_SEMIPLANAR,
            -> i420ToNv12(i420, imageProxy.width, imageProxy.height)

            else -> i420
        }
    }

    private fun yuv420888ToI420(imageProxy: ImageProxy): ByteArray {
        val width = imageProxy.width
        val height = imageProxy.height
        val ySize = width * height
        val uvSize = ySize / 4

        val output = ByteArray(ySize + uvSize * 2)
        copyPlane(imageProxy.planes[0], width, height, output, 0)
        copyPlane(imageProxy.planes[1], width / 2, height / 2, output, ySize)
        copyPlane(imageProxy.planes[2], width / 2, height / 2, output, ySize + uvSize)

        return output
    }

    private fun copyPlane(
        plane: ImageProxy.PlaneProxy,
        width: Int,
        height: Int,
        output: ByteArray,
        outputOffset: Int,
    ) {
        val buffer = plane.buffer
        val rowStride = plane.rowStride
        val pixelStride = plane.pixelStride
        val rowData = ByteArray(rowStride)

        var outOffset = outputOffset
        val initialPosition = buffer.position()

        for (row in 0 until height) {
            val length = if (pixelStride == 1) width else (width - 1) * pixelStride + 1
            buffer.get(rowData, 0, length)

            if (pixelStride == 1) {
                System.arraycopy(rowData, 0, output, outOffset, width)
                outOffset += width
            } else {
                for (col in 0 until width) {
                    output[outOffset++] = rowData[col * pixelStride]
                }
            }

            if (row < height - 1) {
                buffer.position(buffer.position() + rowStride - length)
            }
        }

        buffer.position(initialPosition)
    }

    private fun i420ToNv12(i420: ByteArray, width: Int, height: Int): ByteArray {
        val ySize = width * height
        val uvSize = ySize / 4

        val output = ByteArray(ySize + uvSize * 2)
        System.arraycopy(i420, 0, output, 0, ySize)

        var uIndex = ySize
        var vIndex = ySize + uvSize
        var outIndex = ySize
        for (i in 0 until uvSize) {
            output[outIndex++] = i420[uIndex++]
            output[outIndex++] = i420[vIndex++]
        }

        return output
    }

    companion object {
        private const val TAG = "H264Encoder"
        private const val MIME_TYPE = MediaFormat.MIMETYPE_VIDEO_AVC
        private const val COLOR_TI_FORMAT_YUV420_PACKED_SEMIPLANAR = 0x7F000100
    }
}

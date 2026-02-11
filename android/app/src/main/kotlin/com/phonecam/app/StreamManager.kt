package com.phonecam.app

import android.util.Log
import android.util.Size
import androidx.camera.core.ImageProxy
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.NativeLong

enum class StreamResolution(
    val width: Int,
    val height: Int,
) {
    HD_720P(width = 1280, height = 720),
    FULL_HD_1080P(width = 1920, height = 1080),
}

data class StreamConfig(
    val endpointHost: String = "10.0.2.2",
    val endpointPort: Int = 7878,
    val resolution: StreamResolution = StreamResolution.HD_720P,
    val bitRate: Int = 4_000_000,
    val fps: Int = 30,
) {
    val targetSize: Size
        get() = Size(resolution.width, resolution.height)
}

class StreamManager(
    private val config: StreamConfig,
    private val onStatus: (String) -> Unit = {},
) {
    private val encoder =
        H264Encoder(
            width = config.resolution.width,
            height = config.resolution.height,
            bitRate = config.bitRate,
            frameRate = config.fps,
            onNalUnitReady = ::onNalUnit,
        )

    @Volatile
    private var started = false

    fun start() {
        if (started) {
            return
        }

        RustBridge.setVideoResolution(config.resolution.width, config.resolution.height)
        val connected = RustBridge.initializeTransport(config.endpointHost, config.endpointPort)
        if (!connected) {
            onStatus("Transport connection failed. Camera preview/encoding still active.")
        } else {
            onStatus("Connected to transport ${config.endpointHost}:${config.endpointPort}")
        }

        encoder.start()
        started = true
    }

    fun stop() {
        if (!started) {
            return
        }

        started = false
        runCatching {
            encoder.stop()
        }.onFailure {
            Log.w(TAG, "Failed stopping encoder", it)
        }
        RustBridge.shutdownTransport()
        onStatus("Streaming stopped")
    }

    fun handleCameraFrame(imageProxy: ImageProxy) {
        try {
            if (!started) {
                return
            }

            encoder.encode(imageProxy)
        } catch (t: Throwable) {
            Log.e(TAG, "Failed to encode/send frame", t)
            onStatus("Frame pipeline error: ${t.message ?: "unknown"}")
        } finally {
            imageProxy.close()
        }
    }

    fun targetResolution(): Size = config.targetSize

    private fun onNalUnit(
        nalUnit: ByteArray,
        ptsUs: Long,
        isKeyframe: Boolean,
    ) {
        RustBridge.sendVideoFrame(nalUnit, ptsUs, isKeyframe)
    }

    private interface PhoneCamRustLib : Library {
        fun phonecam_transport_init(host: String, port: Short): Boolean

        fun phonecam_transport_shutdown()

        fun phonecam_set_video_resolution(width: Short, height: Short)

        fun phonecam_send_video_frame(
            data: ByteArray,
            len: NativeLong,
            pts: Long,
            isKeyframe: Boolean,
        )
    }

    private object RustBridge {
        private val lib: PhoneCamRustLib by lazy {
            Native.load("phonecam_mobile_core", PhoneCamRustLib::class.java)
        }

        fun initializeTransport(
            host: String,
            port: Int,
        ): Boolean {
            if (host.isBlank()) {
                return false
            }

            val safePort = port.coerceIn(1, 65535)
            return runCatching {
                lib.phonecam_transport_init(host, safePort.toShort())
            }.onFailure {
                Log.e(TAG, "Rust transport init failed", it)
            }.getOrDefault(false)
        }

        fun shutdownTransport() {
            runCatching {
                lib.phonecam_transport_shutdown()
            }.onFailure {
                Log.w(TAG, "Rust transport shutdown failed", it)
            }
        }

        fun setVideoResolution(
            width: Int,
            height: Int,
        ) {
            runCatching {
                lib.phonecam_set_video_resolution(width.toShort(), height.toShort())
            }.onFailure {
                Log.w(TAG, "Failed to set Rust video resolution metadata", it)
            }
        }

        fun sendVideoFrame(
            nalUnit: ByteArray,
            ptsUs: Long,
            isKeyframe: Boolean,
        ) {
            if (nalUnit.isEmpty()) {
                return
            }

            runCatching {
                lib.phonecam_send_video_frame(
                    nalUnit,
                    NativeLong(nalUnit.size.toLong()),
                    ptsUs,
                    isKeyframe,
                )
            }.onFailure {
                Log.w(TAG, "Rust video frame send failed", it)
            }
        }
    }

    companion object {
        private const val TAG = "StreamManager"
    }
}

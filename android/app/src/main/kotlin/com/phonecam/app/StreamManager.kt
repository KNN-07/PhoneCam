package com.phonecam.app

import android.util.Log
import android.util.Size
import androidx.camera.core.ImageProxy
import androidx.camera.view.PreviewView
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.NativeLong
import com.sun.jna.Pointer
import java.util.concurrent.Executors
import java.util.concurrent.ScheduledExecutorService
import java.util.concurrent.TimeUnit

enum class StreamResolution(
    val width: Int,
    val height: Int,
) {
    SD_480P(width = 640, height = 480),
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

data class QrConnectionInfo(
    val host: String,
    val port: Int,
    val deviceName: String,
)

class StreamManager(
    private val config: StreamConfig,
    private val onStatus: (String) -> Unit = {},
) {
    @Volatile
    private var activeResolution = config.resolution
    @Volatile
    private var activeFps = config.fps
    @Volatile
    private var encoder = createEncoder(activeResolution, activeFps)

    private fun createEncoder(
        resolution: StreamResolution,
        fps: Int,
    ) =
        H264Encoder(
            width = resolution.width,
            height = resolution.height,
            bitRate = bitrateFor(resolution),
            frameRate = fps,
            onNalUnitReady = ::onNalUnit,
        )

    @Volatile
    private var started = false
    @Volatile
    private var isFrontCamera = false
    @Volatile
    private var commandPoller: ScheduledExecutorService? = null
    @Volatile
    private var switchInProgress = false
    @Volatile
    private var cameraController: CameraController? = null
    @Volatile
    private var previewView: PreviewView? = null
    @Volatile
    private var onCameraStateChanged: ((Boolean) -> Unit)? = null

    private val cameraSwitchLock = Any()

    fun registerCameraPipeline(
        cameraController: CameraController,
        previewView: PreviewView,
    ) {
        this.cameraController = cameraController
        this.previewView = previewView
        this.isFrontCamera = cameraController.isUsingFrontCamera()
        notifyCameraStateChanged(this.isFrontCamera)
    }

    fun setOnCameraStateChanged(listener: (Boolean) -> Unit) {
        onCameraStateChanged = listener
        listener(isFrontCamera)
    }

    fun start() {
        if (started) {
            return
        }

        RustBridge.setVideoResolution(activeResolution.width, activeResolution.height)
        val connected = RustBridge.initializeTransport(config.endpointHost, config.endpointPort)
        if (!connected) {
            onStatus("Transport connection failed. Camera preview/encoding still active.")
        } else {
            onStatus("Connected to transport ${config.endpointHost}:${config.endpointPort}")
        }

        encoder.start()
        started = true
        startCameraControlPolling()
    }

    fun stop() {
        if (!started) {
            return
        }

        started = false
        stopCameraControlPolling()
        synchronized(cameraSwitchLock) {
            switchInProgress = false
        }
        runCatching {
            encoder.stop()
        }.onFailure {
            Log.w(TAG, "Failed stopping encoder", it)
        }
        RustBridge.shutdownTransport()
        isFrontCamera = false
        notifyCameraStateChanged(isFrontCamera)
        onStatus("Streaming stopped")
    }

    fun reconnect(
        host: String,
        port: Int,
    ): Boolean {
        if (host.isBlank()) {
            return false
        }

        val safePort = port.coerceIn(1, 65535)
        val connected = RustBridge.reconnectTransport(host.trim(), safePort)
        if (connected) {
            onStatus("Connected to transport ${host.trim()}:$safePort")
        } else {
            onStatus("Unable to connect to transport ${host.trim()}:$safePort")
        }
        return connected
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

    fun targetResolution(): Size = Size(activeResolution.width, activeResolution.height)

    fun targetFps(): Int = activeFps

    private fun notifyCameraStateChanged(frontCamera: Boolean) {
        runCatching {
            onCameraStateChanged?.invoke(frontCamera)
        }.onFailure {
            Log.w(TAG, "Camera state listener failed", it)
        }
    }

    private fun startCameraControlPolling() {
        stopCameraControlPolling()

        commandPoller =
            Executors.newSingleThreadScheduledExecutor().also { executor ->
                executor.scheduleAtFixedRate(
                    {
                        if (!started) {
                            return@scheduleAtFixedRate
                        }

                        when (val command = RustBridge.pollControlCommand() ?: return@scheduleAtFixedRate) {
                            is RemoteControl.SwitchCamera -> handleRemoteCameraSwitch(command.front)
                            RemoteControl.RequestKeyframe -> encoder.requestKeyFrame()
                            is RemoteControl.ConfigureStream -> {
                                handleRemoteStreamConfiguration(command.width, command.height, command.fps)
                            }
                        }
                    },
                    0,
                    CAMERA_CONTROL_POLL_INTERVAL_MS,
                    TimeUnit.MILLISECONDS,
                )
            }
    }

    private fun stopCameraControlPolling() {
        commandPoller?.shutdownNow()
        commandPoller = null
    }

    private fun handleRemoteCameraSwitch(requestFrontCamera: Boolean) {
        if (!started) {
            return
        }

        synchronized(cameraSwitchLock) {
            if (switchInProgress) {
                return
            }
            switchInProgress = true
        }

        try {
            performRemoteCameraSwitch(requestFrontCamera)
        } finally {
            synchronized(cameraSwitchLock) {
                switchInProgress = false
            }
        }
    }

    private fun performRemoteCameraSwitch(requestFrontCamera: Boolean) {
        val controller = cameraController
        val preview = previewView
        if (controller == null || preview == null) {
            onStatus("Camera switch requested before camera pipeline initialization")
            return
        }

        if (requestFrontCamera == isFrontCamera) {
            return
        }

        onStatus("Switching to ${if (requestFrontCamera) "front" else "back"} camera…")

        runCatching {
            encoder.stop()
        }.onFailure {
            Log.w(TAG, "Failed to pause encoder for camera switch", it)
        }

        val actualFrontCamera =
            runCatching {
                controller.switchCamera(
                    useFrontCamera = requestFrontCamera,
                    previewView = preview,
                    targetResolution = targetResolution(),
                    targetFps = targetFps(),
                    onFrame = ::handleCameraFrame,
                )
            }.onFailure {
                Log.e(TAG, "Camera switch failed", it)
            }.getOrNull()

        if (actualFrontCamera == null) {
            runCatching {
                encoder.start()
                encoder.requestKeyFrame()
            }.onFailure {
                Log.e(TAG, "Failed to recover encoder after camera switch failure", it)
            }
            onStatus("Camera switch failed")
            return
        }

        val encoderRestarted =
            runCatching {
                encoder.start()
                encoder.requestKeyFrame()
            }.onFailure {
                Log.e(TAG, "Failed to restart encoder after camera switch", it)
            }.isSuccess

        if (!encoderRestarted) {
            onStatus("Encoder restart failed after camera switch")
            return
        }

        isFrontCamera = actualFrontCamera
        notifyCameraStateChanged(isFrontCamera)

        if (actualFrontCamera != requestFrontCamera) {
            onStatus(
                "Requested ${if (requestFrontCamera) "front" else "back"} camera unavailable; using ${if (actualFrontCamera) "front" else "back"}",
            )
        } else {
            onStatus("Switched to ${if (actualFrontCamera) "front" else "back"} camera")
        }
    }

    private fun handleRemoteStreamConfiguration(
        width: Int,
        height: Int,
        fps: Int,
    ) {
        val resolution = StreamResolution.entries.firstOrNull { it.width == width && it.height == height }
        if (resolution == null || fps !in SUPPORTED_FRAME_RATES) {
            onStatus("Desktop requested unsupported stream configuration ${width}x$height@$fps")
            return
        }

        if (resolution == activeResolution && fps == activeFps) {
            encoder.requestKeyFrame()
            return
        }

        synchronized(cameraSwitchLock) {
            if (switchInProgress) {
                return
            }
            switchInProgress = true
        }

        try {
            performRemoteStreamConfiguration(resolution, fps)
        } finally {
            synchronized(cameraSwitchLock) {
                switchInProgress = false
            }
        }
    }

    private fun performRemoteStreamConfiguration(
        resolution: StreamResolution,
        fps: Int,
    ) {
        val controller = cameraController
        val preview = previewView
        if (controller == null || preview == null) {
            onStatus("Stream configuration requested before camera pipeline initialization")
            return
        }

        val previousResolution = activeResolution
        val previousFps = activeFps
        runCatching { encoder.stop() }

        val configured =
            runCatching {
                activeResolution = resolution
                activeFps = fps
                controller.switchCamera(
                    useFrontCamera = isFrontCamera,
                    previewView = preview,
                    targetResolution = targetResolution(),
                    targetFps = targetFps(),
                    onFrame = ::handleCameraFrame,
                )
                encoder = createEncoder(activeResolution, activeFps)
                encoder.start()
                encoder.requestKeyFrame()
                RustBridge.setVideoResolution(activeResolution.width, activeResolution.height)
            }.onFailure {
                Log.e(TAG, "Failed to apply remote stream configuration", it)
            }.isSuccess

        if (configured) {
            onStatus("Streaming ${resolution.width}x${resolution.height}@$fps")
            return
        }

        activeResolution = previousResolution
        activeFps = previousFps
        runCatching {
            controller.switchCamera(
                useFrontCamera = isFrontCamera,
                previewView = preview,
                targetResolution = targetResolution(),
                targetFps = targetFps(),
                onFrame = ::handleCameraFrame,
            )
            encoder = createEncoder(activeResolution, activeFps)
            encoder.start()
            encoder.requestKeyFrame()
            RustBridge.setVideoResolution(activeResolution.width, activeResolution.height)
        }
        onStatus("Unable to apply stream configuration; restored previous settings")
    }

    private fun bitrateFor(resolution: StreamResolution): Int =
        when (resolution) {
            StreamResolution.SD_480P -> minOf(config.bitRate, 2_500_000)
            StreamResolution.HD_720P -> config.bitRate
            StreamResolution.FULL_HD_1080P -> maxOf(config.bitRate, 5_000_000)
        }

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

        fun phonecam_parse_qr_code_uri(uri: String): Pointer?

        fun phonecam_poll_control_command(): Long

        fun phonecam_discover_desktops(timeoutMs: Int): Pointer?

        fun phonecam_string_free(ptr: Pointer?)
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

        fun reconnectTransport(
            host: String,
            port: Int,
        ): Boolean {
            shutdownTransport()
            return initializeTransport(host, port)
        }

        fun parseQrConnectionUri(uri: String): QrConnectionInfo? {
            val ptr =
                runCatching {
                    lib.phonecam_parse_qr_code_uri(uri)
                }.onFailure {
                    Log.w(TAG, "Rust QR URI parse call failed", it)
                }.getOrNull() ?: return null

            return try {
                val payload = ptr.getString(0)
                val parts = payload.split('|', limit = 3)
                if (parts.size != 3) {
                    null
                } else {
                    val host = parts[0].trim()
                    val port = parts[1].toIntOrNull()?.coerceIn(1, 65535)
                    val deviceName = parts[2].trim().ifBlank { "PhoneCam Desktop" }

                    if (host.isBlank() || port == null) {
                        null
                    } else {
                        QrConnectionInfo(host = host, port = port, deviceName = deviceName)
                    }
                }
            } finally {
                runCatching {
                    lib.phonecam_string_free(ptr)
                }.onFailure {
                    Log.w(TAG, "Failed to free Rust QR parse response", it)
                }
            }
        }

        fun discoverDesktops(timeoutMs: Int = 3_000): List<QrConnectionInfo> {
            val ptr =
                runCatching {
                    lib.phonecam_discover_desktops(timeoutMs.coerceIn(100, 5_000))
                }.onFailure {
                    Log.w(TAG, "Rust mDNS discovery failed", it)
                }.getOrNull() ?: return emptyList()

            return try {
                ptr.getString(0)
                    .lineSequence()
                    .filter { it.isNotBlank() }
                    .mapNotNull { record ->
                        val parts = record.split('|', limit = 4)
                        if (parts.size != 4) {
                            return@mapNotNull null
                        }
                        val port = parts[2].toIntOrNull()?.coerceIn(1, 65535) ?: return@mapNotNull null
                        QrConnectionInfo(
                            host = parts[1],
                            port = port,
                            deviceName = parts[0].ifBlank { "PhoneCam Desktop" },
                        )
                    }.toList()
            } finally {
                runCatching { lib.phonecam_string_free(ptr) }
            }
        }

        fun pollControlCommand(): RemoteControl? {
            val packed =
                runCatching {
                    lib.phonecam_poll_control_command()
                }.onFailure {
                    Log.w(TAG, "Rust camera control poll failed", it)
                }.getOrDefault(CONTROL_NONE)

            return when ((packed and 0xffL).toInt()) {
                CONTROL_SWITCH_FRONT -> RemoteControl.SwitchCamera(front = true)
                CONTROL_SWITCH_BACK -> RemoteControl.SwitchCamera(front = false)
                CONTROL_REQUEST_KEYFRAME -> RemoteControl.RequestKeyframe
                CONTROL_CONFIGURE_STREAM ->
                    RemoteControl.ConfigureStream(
                        width = ((packed ushr 8) and 0xffffL).toInt(),
                        height = ((packed ushr 24) and 0xffffL).toInt(),
                        fps = ((packed ushr 40) and 0xffL).toInt(),
                    )
                else -> null
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
        private const val CAMERA_CONTROL_POLL_INTERVAL_MS = 150L
        private const val CONTROL_NONE = 0L
        private const val CONTROL_SWITCH_BACK = 1
        private const val CONTROL_SWITCH_FRONT = 2
        private const val CONTROL_REQUEST_KEYFRAME = 3
        private const val CONTROL_CONFIGURE_STREAM = 4
        private val SUPPORTED_FRAME_RATES = setOf(15, 30, 60)

        fun parseQrConnectionUri(uri: String): QrConnectionInfo? = RustBridge.parseQrConnectionUri(uri)

        fun discoverDesktops(): List<QrConnectionInfo> = RustBridge.discoverDesktops()
    }
}

private sealed class RemoteControl {
    data class SwitchCamera(
        val front: Boolean,
    ) : RemoteControl()

    data object RequestKeyframe : RemoteControl()

    data class ConfigureStream(
        val width: Int,
        val height: Int,
        val fps: Int,
    ) : RemoteControl()
}

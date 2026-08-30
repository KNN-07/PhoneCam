package com.phonecam.app

import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.util.Log
import android.util.Size
import androidx.camera.core.ImageProxy
import androidx.camera.view.PreviewView
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.NativeLong
import com.sun.jna.Pointer
import org.json.JSONArray
import org.json.JSONObject
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
    QHD_1440P(width = 2560, height = 1440),
    UHD_2160P(width = 3840, height = 2160),
}

enum class VideoCodec(
    val wireName: String,
    val nativeId: Byte,
) {
    H264(wireName = "h264", nativeId = 0),
    HEVC(wireName = "hevc", nativeId = 1),
}

data class StreamProfile(
    val codec: VideoCodec,
    val width: Int,
    val height: Int,
    val fps: Int,
) {
    fun toJson(): JSONObject =
        JSONObject()
            .put("codec", codec.wireName)
            .put("width", width)
            .put("height", height)
            .put("fps", fps)

    companion object {
        fun fromJson(json: JSONObject): StreamProfile? {
            val codec =
                VideoCodec.entries.firstOrNull {
                    it.wireName == json.optString("codec")
                } ?: return null
            val profile =
                StreamProfile(
                    codec = codec,
                    width = json.optInt("width"),
                    height = json.optInt("height"),
                    fps = json.optInt("fps"),
                )
            return profile.takeIf {
                StreamResolution.entries.any { resolution ->
                    resolution.width == profile.width && resolution.height == profile.height
                } && profile.fps in setOf(15, 30, 60)
            }
        }
    }
}

data class StreamConfig(
    val endpointHost: String = "10.0.2.2",
    val endpointPort: Int = 7878,
    val resolution: StreamResolution = StreamResolution.HD_720P,
    val bitRateOverride: Int? = null,
    val fps: Int = 30,
    val codec: VideoCodec = VideoCodec.H264,
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
    private var activeCodec = config.codec
    @Volatile
    private var encoder = createEncoder(activeCodec, activeResolution, activeFps)

    private fun createEncoder(
        codec: VideoCodec,
        resolution: StreamResolution,
        fps: Int,
    ) =
        VideoEncoder(
            codec = codec,
            width = resolution.width,
            height = resolution.height,
            bitRate = bitrateFor(codec, resolution, fps, config.bitRateOverride),
            frameRate = fps,
            onNalUnitReady = ::onNalUnit,
        )
    private fun activeProfile(): StreamProfile =
        StreamProfile(
            codec = activeCodec,
            width = activeResolution.width,
            height = activeResolution.height,
            fps = activeFps,
        )

    @Volatile
    private var availableProfiles: List<StreamProfile> = listOf(activeProfile())

    private fun supportedProfiles(): List<StreamProfile> = availableProfiles


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
        availableProfiles =
            discoverSupportedProfiles(cameraController.discoverExactCaptureProfiles())
                .ifEmpty { listOf(activeProfile()) }
        require(activeProfile() in availableProfiles) {
            "Startup profile ${activeProfile()} is unsupported by the camera and hardware encoders"
        }
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

        val connected =
            RustBridge.initializeTransport(
                config.endpointHost,
                config.endpointPort,
                activeProfile(),
                supportedProfiles(),
            )
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
        val connected =
            RustBridge.reconnectTransport(
                host.trim(),
                safePort,
                activeProfile(),
                supportedProfiles(),
            )
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
                                handleRemoteStreamConfiguration(command.requestId, command.profile)
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
        requestId: Int,
        profile: StreamProfile,
    ) {
        val resolution =
            StreamResolution.entries.firstOrNull {
                it.width == profile.width && it.height == profile.height
            }
        if (resolution == null ||
            profile.fps !in SUPPORTED_FRAME_RATES ||
            profile !in supportedProfiles()
        ) {
            RustBridge.reportConfiguration(requestId, RESULT_UNSUPPORTED, profile)
            onStatus(
                "Desktop requested unsupported stream configuration " +
                    "${profile.codec.wireName} ${profile.width}x${profile.height}@${profile.fps}",
            )
            return
        }
        if (profile == activeProfile()) {
            if (RustBridge.reportConfiguration(requestId, RESULT_APPLIED, profile)) {
                encoder.requestKeyFrame()
            }
            return
        }
        synchronized(cameraSwitchLock) {
            if (switchInProgress) {
                RustBridge.reportConfiguration(requestId, RESULT_UNSUPPORTED, profile)
                return
            }
            switchInProgress = true
        }
        try {
            val applied =
                configurationCandidates(profile, supportedProfiles())
                    .firstNotNullOfOrNull { candidate ->
                        candidate.takeIf { started }?.let(::performRemoteStreamConfiguration)
                    }
            if (applied != null) {
                if (RustBridge.reportConfiguration(requestId, RESULT_APPLIED, applied)) {
                    encoder.requestKeyFrame()
                    onStatus(
                        "Streaming ${applied.codec.wireName} " +
                            "${applied.width}x${applied.height}@${applied.fps}",
                    )
                }
            } else {
                RustBridge.reportConfiguration(requestId, RESULT_CAPTURE_FAILED, profile)
            }
        } finally {
            synchronized(cameraSwitchLock) {
                switchInProgress = false
            }
        }
    }

    private fun performRemoteStreamConfiguration(profile: StreamProfile): StreamProfile? {
        val controller = cameraController
        val preview = previewView
        if (controller == null || preview == null) {
            onStatus("Stream configuration requested before camera pipeline initialization")
            return null
        }
        val resolution =
            StreamResolution.entries.firstOrNull {
                it.width == profile.width && it.height == profile.height
            } ?: return null
        val previousProfile = activeProfile()
        val previousEncoder = encoder
        val candidate = createEncoder(profile.codec, resolution, profile.fps)
        runCatching { previousEncoder.stop() }
        val configured =
            runCatching {
                controller.switchCamera(
                    useFrontCamera = isFrontCamera,
                    previewView = preview,
                    targetResolution = Size(profile.width, profile.height),
                    targetFps = profile.fps,
                    onFrame = ::handleCameraFrame,
                )
                candidate.start()
            }.onFailure {
                Log.e(TAG, "Failed to apply remote stream configuration", it)
            }.isSuccess
        if (configured) {
            activeCodec = profile.codec
            activeResolution = resolution
            activeFps = profile.fps
            encoder = candidate
            return profile
        }

        runCatching { candidate.stop() }
        availableProfiles = availableProfiles - profile
        RustBridge.updateVideoCapabilities(availableProfiles)
        val restoredResolution =
            StreamResolution.entries.first {
                it.width == previousProfile.width && it.height == previousProfile.height
            }
        val restored =
            runCatching {
                controller.switchCamera(
                    useFrontCamera = isFrontCamera,
                    previewView = preview,
                    targetResolution = Size(previousProfile.width, previousProfile.height),
                    targetFps = previousProfile.fps,
                    onFrame = ::handleCameraFrame,
                )
                val restoredEncoder =
                    createEncoder(
                        previousProfile.codec,
                        restoredResolution,
                        previousProfile.fps,
                    )
                restoredEncoder.start()
                activeCodec = previousProfile.codec
                activeResolution = restoredResolution
                activeFps = previousProfile.fps
                encoder = restoredEncoder
                encoder.requestKeyFrame()
            }.onFailure {
                Log.e(TAG, "Failed to restore previous stream configuration", it)
            }.isSuccess
        if (!restored) {
            started = false
            stopCameraControlPolling()
            runCatching { controller.stop() }
            onStatus("Terminal stream failure: unable to restore previous configuration")
            return null
        }
        onStatus("Unable to apply stream configuration; restored previous settings")
        return null
    }

    private fun bitrateFor(
        codec: VideoCodec,
        resolution: StreamResolution,
        fps: Int,
        override: Int?,
    ): Int {
        val default = defaultBitrate(codec, resolution, fps)
        return override?.coerceIn(MIN_BITRATE_OVERRIDE, MAX_BITRATE_OVERRIDE) ?: default
    }

    private fun onNalUnit(
        nalUnit: ByteArray,
        ptsUs: Long,
        isKeyframe: Boolean,
    ) {
        val accepted = RustBridge.sendVideoFrame(nalUnit, ptsUs, activeProfile(), isKeyframe)
        if (!accepted && isKeyframe) {
            encoder.requestKeyFrame()
        }
    }

    private interface PhoneCamRustLib : Library {
        fun phonecam_transport_init(
            host: String,
            port: Short,
            videoConfigJson: String,
        ): Boolean

        fun phonecam_transport_shutdown()

        fun phonecam_send_video_frame(
            data: ByteArray,
            len: NativeLong,
            pts: Long,
            codec: Byte,
            width: Short,
            height: Short,
            isKeyframe: Boolean,
        ): Boolean

        fun phonecam_poll_control_command_json(): Pointer?

        fun phonecam_peer_supports_profile(
            codec: Byte,
            width: Short,
            height: Short,
            fps: Byte,
        ): Boolean

        fun phonecam_update_video_capabilities(profilesJson: String): Boolean

        fun phonecam_report_stream_configuration(
            requestId: Int,
            resultCode: Byte,
            codec: Byte,
            width: Short,
            height: Short,
            fps: Byte,
        ): Boolean

        fun phonecam_parse_qr_code_uri(uri: String): Pointer?

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
            activeProfile: StreamProfile,
            supportedProfiles: List<StreamProfile>,
        ): Boolean {
            if (host.isBlank() || supportedProfiles.isEmpty() || activeProfile !in supportedProfiles) {
                return false
            }
            val profilesJson = JSONArray()
            supportedProfiles.forEach { profilesJson.put(it.toJson()) }
            val configJson =
                JSONObject()
                    .put("active_profile", activeProfile.toJson())
                    .put("supported_profiles", profilesJson)
                    .toString()
            val safePort = port.coerceIn(1, 65535)
            return runCatching {
                lib.phonecam_transport_init(host, safePort.toShort(), configJson)
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
        fun updateVideoCapabilities(profiles: List<StreamProfile>): Boolean {
            if (profiles.isEmpty()) {
                return false
            }
            val json = JSONArray()
            profiles.forEach { json.put(it.toJson()) }
            return runCatching {
                lib.phonecam_update_video_capabilities(json.toString())
            }.onFailure {
                Log.w(TAG, "Rust capability update failed", it)
            }.getOrDefault(false)
        }


        fun reconnectTransport(
            host: String,
            port: Int,
            activeProfile: StreamProfile,
            supportedProfiles: List<StreamProfile>,
        ): Boolean {
            shutdownTransport()
            return initializeTransport(host, port, activeProfile, supportedProfiles)
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
            val ptr =
                runCatching {
                    lib.phonecam_poll_control_command_json()
                }.onFailure {
                    Log.w(TAG, "Rust camera control poll failed", it)
                }.getOrNull() ?: return null
            return try {
                val json = JSONObject(ptr.getString(0))
                when (json.optString("type")) {
                    "switch_camera" -> RemoteControl.SwitchCamera(json.getBoolean("front"))
                    "request_keyframe" -> RemoteControl.RequestKeyframe
                    "configure_stream" -> {
                        val profile = StreamProfile.fromJson(json.getJSONObject("profile")) ?: return null
                        RemoteControl.ConfigureStream(
                            requestId = json.getLong("request_id").toInt(),
                            profile = profile,
                        )
                    }
                    else -> null
                }
            } catch (error: Exception) {
                Log.w(TAG, "Invalid Rust control JSON", error)
                null
            } finally {
                runCatching { lib.phonecam_string_free(ptr) }
            }
        }

        fun reportConfiguration(
            requestId: Int,
            resultCode: Byte,
            profile: StreamProfile,
        ): Boolean =
            runCatching {
                lib.phonecam_report_stream_configuration(
                    requestId,
                    resultCode,
                    profile.codec.nativeId,
                    profile.width.toShort(),
                    profile.height.toShort(),
                    profile.fps.toByte(),
                )
            }.getOrDefault(false)

        fun sendVideoFrame(
            nalUnit: ByteArray,
            ptsUs: Long,
            profile: StreamProfile,
            isKeyframe: Boolean,
        ): Boolean {
            if (nalUnit.isEmpty()) {
                return false
            }
            return runCatching {
                lib.phonecam_send_video_frame(
                    nalUnit,
                    NativeLong(nalUnit.size.toLong()),
                    ptsUs,
                    profile.codec.nativeId,
                    profile.width.toShort(),
                    profile.height.toShort(),
                    isKeyframe,
                )
            }.onFailure {
                Log.w(TAG, "Rust video frame send failed", it)
            }.getOrDefault(false)
        }
    }

    companion object {
        private const val TAG = "StreamManager"
        private const val CAMERA_CONTROL_POLL_INTERVAL_MS = 150L
        private const val RESULT_APPLIED: Byte = 0
        private const val RESULT_UNSUPPORTED: Byte = 1
        private const val RESULT_CAPTURE_FAILED: Byte = 2
        private val SUPPORTED_FRAME_RATES = setOf(15, 30, 60)
        private const val MIN_BITRATE_OVERRIDE = 500_000
        private const val MAX_BITRATE_OVERRIDE = 80_000_000

        internal fun configurationCandidates(
            requested: StreamProfile,
            supported: List<StreamProfile>,
        ): List<StreamProfile> {
            val candidates = mutableListOf(requested)
            if (requested.codec == VideoCodec.HEVC) {
                requested.copy(codec = VideoCodec.H264)
                    .takeIf { it in supported }
                    ?.let(candidates::add)
            }
            return candidates
        }

        internal fun defaultBitrate(
            codec: VideoCodec,
            resolution: StreamResolution,
            fps: Int,
        ): Int {
            val rateIndex =
                when (fps) {
                    15 -> 0
                    30 -> 1
                    60 -> 2
                    else -> throw IllegalArgumentException("Unsupported frame rate: $fps")
                }
            val rates =
                when (codec) {
                    VideoCodec.H264 ->
                        when (resolution) {
                            StreamResolution.SD_480P -> intArrayOf(1_000_000, 2_000_000, 3_000_000)
                            StreamResolution.HD_720P -> intArrayOf(2_500_000, 4_000_000, 7_000_000)
                            StreamResolution.FULL_HD_1080P -> intArrayOf(4_000_000, 8_000_000, 12_000_000)
                            StreamResolution.QHD_1440P -> intArrayOf(8_000_000, 16_000_000, 24_000_000)
                            StreamResolution.UHD_2160P -> intArrayOf(16_000_000, 35_000_000, 50_000_000)
                        }
                    VideoCodec.HEVC ->
                        when (resolution) {
                            StreamResolution.SD_480P -> intArrayOf(750_000, 1_250_000, 2_000_000)
                            StreamResolution.HD_720P -> intArrayOf(1_500_000, 2_500_000, 4_500_000)
                            StreamResolution.FULL_HD_1080P -> intArrayOf(2_500_000, 5_000_000, 8_000_000)
                            StreamResolution.QHD_1440P -> intArrayOf(5_000_000, 9_000_000, 15_000_000)
                            StreamResolution.UHD_2160P -> intArrayOf(10_000_000, 20_000_000, 32_000_000)
                        }
                }
            return rates[rateIndex]
        }

        internal fun intersectProfiles(
            captureProfiles: Set<Pair<StreamResolution, Int>>,
            encoderSupports: (StreamProfile, Int) -> Boolean,
        ): List<StreamProfile> =
            captureProfiles
                .flatMap { (resolution, fps) ->
                    VideoCodec.entries.mapNotNull { codec ->
                        val profile =
                            StreamProfile(codec, resolution.width, resolution.height, fps)
                        profile.takeIf {
                            encoderSupports(profile, defaultBitrate(codec, resolution, fps))
                        }
                    }
                }.sortedWith(
                    compareBy<StreamProfile>(
                        { it.width * it.height },
                        { it.fps },
                        { it.codec.nativeId },
                    ),
                )

        private fun discoverSupportedProfiles(
            captureProfiles: Set<Pair<StreamResolution, Int>>,
        ): List<StreamProfile> {
            val codecInfos =
                MediaCodecList(MediaCodecList.ALL_CODECS).codecInfos
                    .asSequence()
                    .filter(MediaCodecInfo::isEncoder)
                    .sortedByDescending {
                        if (android.os.Build.VERSION.SDK_INT >= 29) it.isHardwareAccelerated else true
                    }.toList()
            return intersectProfiles(captureProfiles) { profile, bitrate ->
                val mime =
                    when (profile.codec) {
                        VideoCodec.H264 -> MediaFormat.MIMETYPE_VIDEO_AVC
                        VideoCodec.HEVC -> MediaFormat.MIMETYPE_VIDEO_HEVC
                    }
                codecInfos.any { info ->
                    mime in info.supportedTypes &&
                        runCatching {
                            val capabilities = info.getCapabilitiesForType(mime)
                            val requiredProfile =
                                when (profile.codec) {
                                    VideoCodec.H264 -> setOf(
                                        MediaCodecInfo.CodecProfileLevel.AVCProfileBaseline,
                                        MediaCodecInfo.CodecProfileLevel.AVCProfileHigh,
                                    )
                                    VideoCodec.HEVC -> setOf(MediaCodecInfo.CodecProfileLevel.HEVCProfileMain)
                                }
                            capabilities.profileLevels.any { it.profile in requiredProfile } &&
                                capabilities.videoCapabilities.areSizeAndRateSupported(
                                    profile.width,
                                    profile.height,
                                    profile.fps.toDouble(),
                                ) &&
                                bitrate in capabilities.videoCapabilities.bitrateRange
                        }.getOrDefault(false)
                }
            }
        }

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
        val requestId: Int,
        val profile: StreamProfile,
    ) : RemoteControl()
}

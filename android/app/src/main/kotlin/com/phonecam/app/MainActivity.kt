package com.phonecam.app

import android.Manifest
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat

class MainActivity : AppCompatActivity() {
    private lateinit var previewView: PreviewView
    private lateinit var statusTextView: TextView
    private lateinit var cameraController: CameraController
    private lateinit var streamManager: StreamManager

    private val cameraPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) {
                startCameraPipeline()
            } else {
                updateStatus("Camera permission denied; preview cannot start")
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        previewView = findViewById(R.id.cameraPreviewView)
        statusTextView = findViewById(R.id.statusTextView)

        val streamConfig =
            StreamConfig(
                endpointHost =
                    intent
                        .getStringExtra(EXTRA_ENDPOINT_HOST)
                        ?.takeIf { it.isNotBlank() }
                        ?: DEFAULT_ENDPOINT_HOST,
                endpointPort =
                    intent
                        .getIntExtra(EXTRA_ENDPOINT_PORT, DEFAULT_ENDPOINT_PORT)
                        .coerceIn(1, 65535),
                resolution = parseResolution(intent.getStringExtra(EXTRA_RESOLUTION)),
                bitRate =
                    intent
                        .getIntExtra(EXTRA_BITRATE, DEFAULT_BITRATE)
                        .coerceIn(MIN_BITRATE, MAX_BITRATE),
                fps =
                    intent
                        .getIntExtra(EXTRA_FPS, DEFAULT_FPS)
                        .coerceIn(24, 60),
            )

        streamManager = StreamManager(streamConfig, ::updateStatus)
        cameraController = CameraController(applicationContext, this)

        updateStatus("Ready. Waiting for camera permission…")
        ensureCameraPermissionAndStart()
    }

    override fun onDestroy() {
        streamManager.stop()
        cameraController.release()
        super.onDestroy()
    }

    private fun ensureCameraPermissionAndStart() {
        val granted =
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.CAMERA,
            ) == PackageManager.PERMISSION_GRANTED

        if (granted) {
            startCameraPipeline()
        } else {
            cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    private fun startCameraPipeline() {
        runCatching {
            streamManager.start()
            cameraController.start(
                previewView = previewView,
                targetResolution = streamManager.targetResolution(),
                onFrame = streamManager::handleCameraFrame,
                onError = {
                    updateStatus("Camera startup failed: ${it.message ?: "unknown error"}")
                },
            )
            val target = streamManager.targetResolution()
            updateStatus("Camera preview active (${target.width}x${target.height})")
        }.onFailure {
            updateStatus("Unable to start camera pipeline: ${it.message ?: "unknown error"}")
            Log.e(TAG, "Camera pipeline start failed", it)
        }
    }

    private fun parseResolution(value: String?): StreamResolution =
        when (value?.lowercase()) {
            "1080p", "fullhd", "full_hd" -> StreamResolution.FULL_HD_1080P
            else -> StreamResolution.HD_720P
        }

    private fun updateStatus(status: String) {
        Log.i(TAG, status)
        runOnUiThread {
            statusTextView.text = status
        }
    }

    companion object {
        private const val TAG = "PhoneCamAndroid"

        private const val DEFAULT_ENDPOINT_HOST = "10.0.2.2"
        private const val DEFAULT_ENDPOINT_PORT = 7878
        private const val DEFAULT_FPS = 30
        private const val DEFAULT_BITRATE = 4_000_000
        private const val MIN_BITRATE = 3_000_000
        private const val MAX_BITRATE = 5_000_000

        const val EXTRA_ENDPOINT_HOST = "phonecam.endpoint_host"
        const val EXTRA_ENDPOINT_PORT = "phonecam.endpoint_port"
        const val EXTRA_RESOLUTION = "phonecam.resolution"
        const val EXTRA_BITRATE = "phonecam.bitrate"
        const val EXTRA_FPS = "phonecam.fps"
    }
}

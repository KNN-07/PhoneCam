package com.phonecam.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.TextView
import androidx.appcompat.app.AlertDialog
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat

class MainActivity : AppCompatActivity() {
    private lateinit var previewView: PreviewView
    private lateinit var statusTextView: TextView
    private lateinit var cameraStateTextView: TextView
    private lateinit var scanQrButton: Button
    private lateinit var discoverButton: Button
    private lateinit var usbConnectButton: Button
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

    private val qrScannerLauncher =
        registerForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
            if (result.resultCode != RESULT_OK) {
                return@registerForActivityResult
            }

            val scannedUri = result.data?.getStringExtra(QRScannerActivity.EXTRA_QR_URI)
            if (scannedUri.isNullOrBlank()) {
                updateStatus("QR scan did not return a PhoneCam URI")
                return@registerForActivityResult
            }

            val qrConnection = StreamManager.parseQrConnectionUri(scannedUri)
            if (qrConnection == null) {
                updateStatus("Scanned QR code is not a valid PhoneCam URI")
                return@registerForActivityResult
            }

            val connected = streamManager.reconnect(qrConnection.host, qrConnection.port)
            if (connected) {
                updateStatus(
                    "Connected to ${qrConnection.deviceName} (${qrConnection.host}:${qrConnection.port}) via QR",
                )
            } else {
                updateStatus(
                    "Unable to connect to ${qrConnection.deviceName} (${qrConnection.host}:${qrConnection.port})",
                )
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_main)

        previewView = findViewById(R.id.cameraPreviewView)
        statusTextView = findViewById(R.id.statusTextView)
        cameraStateTextView = findViewById(R.id.cameraStateTextView)
        scanQrButton = findViewById(R.id.scanQrButton)
        discoverButton = findViewById(R.id.discoverButton)
        usbConnectButton = findViewById(R.id.usbConnectButton)
        scanQrButton.setOnClickListener {
            qrScannerLauncher.launch(Intent(this, QRScannerActivity::class.java))
        }
        discoverButton.setOnClickListener { discoverDesktops() }
        usbConnectButton.setOnClickListener {
            streamManager.reconnect("127.0.0.1", DEFAULT_ENDPOINT_PORT)
        }

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
                        .takeIf { it in SUPPORTED_FRAME_RATES }
                        ?: DEFAULT_FPS,
            )

        streamManager = StreamManager(streamConfig, ::updateStatus)
        streamManager.setOnCameraStateChanged(::updateCameraState)
        cameraController = CameraController(applicationContext, this)

        updateStatus("Ready. Waiting for camera permission…")
        updateCameraState(false)
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
                targetFps = streamManager.targetFps(),
                onFrame = streamManager::handleCameraFrame,
                onError = {
                    updateStatus("Camera startup failed: ${it.message ?: "unknown error"}")
                },
            )
            streamManager.registerCameraPipeline(cameraController, previewView)
            updateCameraState(cameraController.isUsingFrontCamera())
            val target = streamManager.targetResolution()
            updateStatus("Camera preview active (${target.width}x${target.height})")
        }.onFailure {
            updateStatus("Unable to start camera pipeline: ${it.message ?: "unknown error"}")
            Log.e(TAG, "Camera pipeline start failed", it)
        }
    }

    private fun parseResolution(value: String?): StreamResolution =
        when (value?.lowercase()) {
            "480p", "sd" -> StreamResolution.SD_480P
            "1080p", "fullhd", "full_hd" -> StreamResolution.FULL_HD_1080P
            else -> StreamResolution.HD_720P
        }

    private fun discoverDesktops() {
        discoverButton.isEnabled = false
        updateStatus("Discovering PhoneCam desktops…")

        Thread {
            val wifiManager = applicationContext.getSystemService(WIFI_SERVICE) as? WifiManager
            val multicastLock = wifiManager?.createMulticastLock("phonecam-mdns")?.apply {
                setReferenceCounted(false)
                acquire()
            }

            val desktops =
                try {
                    StreamManager.discoverDesktops()
                } finally {
                    multicastLock?.takeIf { it.isHeld }?.release()
                }

            runOnUiThread {
                discoverButton.isEnabled = true
                if (desktops.isEmpty()) {
                    updateStatus("No desktops found; use QR code or manual endpoint settings")
                    return@runOnUiThread
                }

                val labels = desktops.map { "${it.deviceName} (${it.host}:${it.port})" }.toTypedArray()
                AlertDialog.Builder(this)
                    .setTitle("PhoneCam desktops")
                    .setItems(labels) { _, index ->
                        val desktop = desktops[index]
                        streamManager.reconnect(desktop.host, desktop.port)
                    }
                    .setNegativeButton("Cancel", null)
                    .show()
            }
        }.start()
    }

    private fun updateStatus(status: String) {
        Log.i(TAG, status)
        runOnUiThread {
            statusTextView.text = status
        }
    }

    private fun updateCameraState(frontCamera: Boolean) {
        runOnUiThread {
            cameraStateTextView.text = if (frontCamera) "Camera: Front" else "Camera: Back"
        }
    }

    companion object {
        private const val TAG = "PhoneCamAndroid"

        private const val DEFAULT_ENDPOINT_HOST = "127.0.0.1"
        private const val DEFAULT_ENDPOINT_PORT = 7878
        private const val DEFAULT_FPS = 30
        private const val DEFAULT_BITRATE = 4_000_000
        private const val MIN_BITRATE = 3_000_000
        private const val MAX_BITRATE = 5_000_000
        private val SUPPORTED_FRAME_RATES = setOf(15, 30, 60)

        const val EXTRA_ENDPOINT_HOST = "phonecam.endpoint_host"
        const val EXTRA_ENDPOINT_PORT = "phonecam.endpoint_port"
        const val EXTRA_RESOLUTION = "phonecam.resolution"
        const val EXTRA_BITRATE = "phonecam.bitrate"
        const val EXTRA_FPS = "phonecam.fps"
    }
}

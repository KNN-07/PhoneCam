package com.phonecam.app

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Bundle
import android.util.Log
import android.widget.Button
import android.widget.TextView
import androidx.activity.result.contract.ActivityResultContracts
import androidx.appcompat.app.AppCompatActivity
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import com.google.mlkit.vision.barcode.BarcodeScannerOptions
import com.google.mlkit.vision.barcode.BarcodeScanning
import com.google.mlkit.vision.barcode.common.Barcode
import com.google.mlkit.vision.common.InputImage
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

class QRScannerActivity : AppCompatActivity() {
    private lateinit var previewView: PreviewView
    private lateinit var hintTextView: TextView
    private lateinit var cancelButton: Button

    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor()
    private var cameraProvider: ProcessCameraProvider? = null

    @Volatile
    private var didReturnResult = false

    private val scanner by lazy {
        val options =
            BarcodeScannerOptions.Builder()
                .setBarcodeFormats(Barcode.FORMAT_QR_CODE)
                .build()

        BarcodeScanning.getClient(options)
    }

    private val cameraPermissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
            if (granted) {
                startScannerCamera()
            } else {
                hintTextView.text = "Camera permission is required to scan QR codes"
                setResult(RESULT_CANCELED)
                finish()
            }
        }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContentView(R.layout.activity_qr_scanner)

        previewView = findViewById(R.id.qrPreviewView)
        hintTextView = findViewById(R.id.qrScannerHintText)
        cancelButton = findViewById(R.id.qrScannerCancelButton)

        cancelButton.setOnClickListener {
            setResult(RESULT_CANCELED)
            finish()
        }

        ensureCameraPermissionAndStart()
    }

    override fun onDestroy() {
        cameraProvider?.unbindAll()
        runCatching {
            scanner.close()
        }
        analysisExecutor.shutdown()
        super.onDestroy()
    }

    private fun ensureCameraPermissionAndStart() {
        val granted =
            ContextCompat.checkSelfPermission(
                this,
                Manifest.permission.CAMERA,
            ) == PackageManager.PERMISSION_GRANTED

        if (granted) {
            startScannerCamera()
        } else {
            cameraPermissionLauncher.launch(Manifest.permission.CAMERA)
        }
    }

    private fun startScannerCamera() {
        val providerFuture = ProcessCameraProvider.getInstance(this)
        providerFuture.addListener(
            {
                runCatching {
                    val provider = providerFuture.get()
                    cameraProvider = provider

                    val preview =
                        Preview.Builder()
                            .build()
                            .also { it.setSurfaceProvider(previewView.surfaceProvider) }

                    val analyzer =
                        ImageAnalysis.Builder()
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                            .also {
                                it.setAnalyzer(analysisExecutor, ::analyzeFrame)
                            }

                    provider.unbindAll()
                    provider.bindToLifecycle(
                        this,
                        CameraSelector.DEFAULT_BACK_CAMERA,
                        preview,
                        analyzer,
                    )
                }.onFailure {
                    Log.e(TAG, "Failed to start QR scanner camera", it)
                    hintTextView.text = "Unable to start camera for QR scanning"
                }
            },
            ContextCompat.getMainExecutor(this),
        )
    }

    private fun analyzeFrame(imageProxy: ImageProxy) {
        if (didReturnResult) {
            imageProxy.close()
            return
        }

        val mediaImage = imageProxy.image
        if (mediaImage == null) {
            imageProxy.close()
            return
        }

        val inputImage = InputImage.fromMediaImage(mediaImage, imageProxy.imageInfo.rotationDegrees)
        scanner
            .process(inputImage)
            .addOnSuccessListener { barcodes ->
                val matchedUri =
                    barcodes
                        .firstNotNullOfOrNull { barcode ->
                            barcode.rawValue?.takeIf { value -> value.startsWith("phonecam://") }
                        }

                if (!matchedUri.isNullOrBlank() && !didReturnResult) {
                    didReturnResult = true
                    val result =
                        Intent().apply {
                            putExtra(EXTRA_QR_URI, matchedUri)
                        }
                    setResult(RESULT_OK, result)
                    finish()
                }
            }.addOnFailureListener {
                Log.w(TAG, "QR scanner frame processing failed", it)
            }.addOnCompleteListener {
                imageProxy.close()
            }
    }

    companion object {
        private const val TAG = "QRScannerActivity"
        const val EXTRA_QR_URI = "phonecam.qr_uri"
    }
}

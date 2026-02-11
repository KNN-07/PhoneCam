package com.phonecam.app

import android.content.Context
import android.util.Log
import android.util.Size
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.lifecycle.LifecycleOwner
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors

class CameraController(
    private val context: Context,
    private val lifecycleOwner: LifecycleOwner,
) {
    private val mainExecutor = ContextCompat.getMainExecutor(context)
    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    private var cameraProvider: ProcessCameraProvider? = null
    private var activeCamera: Camera? = null

    fun start(
        previewView: PreviewView,
        targetResolution: Size,
        onFrame: (ImageProxy) -> Unit,
        onError: (Throwable) -> Unit = {},
    ) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        providerFuture.addListener(
            {
                runCatching {
                    val provider = providerFuture.get()
                    cameraProvider = provider

                    val preview =
                        Preview.Builder()
                            .setTargetResolution(targetResolution)
                            .build()
                            .also {
                                it.setSurfaceProvider(previewView.surfaceProvider)
                            }

                    val imageAnalysis =
                        ImageAnalysis.Builder()
                            .setTargetResolution(targetResolution)
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                            .also {
                                it.setAnalyzer(analysisExecutor, onFrame)
                            }

                    provider.unbindAll()
                    activeCamera =
                        provider.bindToLifecycle(
                            lifecycleOwner,
                            CameraSelector.DEFAULT_BACK_CAMERA,
                            preview,
                            imageAnalysis,
                        )
                }.onFailure {
                    Log.e(TAG, "Failed to start CameraX", it)
                    onError(it)
                }
            },
            mainExecutor,
        )
    }

    fun stop() {
        cameraProvider?.unbindAll()
        activeCamera = null
    }

    fun release() {
        stop()
        analysisExecutor.shutdown()
    }

    companion object {
        private const val TAG = "CameraController"
    }
}

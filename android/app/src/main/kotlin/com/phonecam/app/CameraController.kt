package com.phonecam.app

import android.content.Context
import android.util.Log
import android.util.Range
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
import java.util.concurrent.CountDownLatch
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class CameraController(
    private val context: Context,
    private val lifecycleOwner: LifecycleOwner,
) {
    private val mainExecutor = ContextCompat.getMainExecutor(context)
    private val analysisExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    private var cameraProvider: ProcessCameraProvider? = null
    private var activeCamera: Camera? = null
    @Volatile
    private var usingFrontCamera: Boolean = false

    fun start(
        previewView: PreviewView,
        targetResolution: Size,
        targetFps: Int,
        onFrame: (ImageProxy) -> Unit,
        onError: (Throwable) -> Unit = {},
    ) {
        val providerFuture = ProcessCameraProvider.getInstance(context)
        providerFuture.addListener(
            {
                runCatching {
                    val provider = providerFuture.get()
                    cameraProvider = provider
                    usingFrontCamera =
                        bindUseCases(
                            provider = provider,
                            useFrontCamera = false,
                            previewView = previewView,
                            targetResolution = targetResolution,
                            targetFps = targetFps,
                            onFrame = onFrame,
                        )
                }.onFailure {
                    Log.e(TAG, "Failed to start CameraX", it)
                    onError(it)
                }
            },
            mainExecutor,
        )
    }

    fun switchCamera(
        useFrontCamera: Boolean,
        previewView: PreviewView,
        targetResolution: Size,
        targetFps: Int,
        onFrame: (ImageProxy) -> Unit,
    ): Boolean {
        val provider = cameraProvider ?: throw IllegalStateException("Camera provider is not initialized")
        val latch = CountDownLatch(1)
        var result: Result<Boolean>? = null

        mainExecutor.execute {
            result =
                runCatching {
                    bindUseCases(
                        provider = provider,
                        useFrontCamera = useFrontCamera,
                        previewView = previewView,
                        targetResolution = targetResolution,
                        targetFps = targetFps,
                        onFrame = onFrame,
                    )
                }
            latch.countDown()
        }

        if (!latch.await(SWITCH_TIMEOUT_MS, TimeUnit.MILLISECONDS)) {
            throw IllegalStateException("Timed out while switching camera")
        }

        return result?.getOrThrow()
            ?: throw IllegalStateException("Camera switch did not complete")
    }

    fun isUsingFrontCamera(): Boolean = usingFrontCamera

    private fun bindUseCases(
        provider: ProcessCameraProvider,
        useFrontCamera: Boolean,
        previewView: PreviewView,
        targetResolution: Size,
        targetFps: Int,
        onFrame: (ImageProxy) -> Unit,
    ): Boolean {
        val cameraSelector = selectCameraSelector(provider, useFrontCamera)

        val preview =
            Preview.Builder()
                .setTargetResolution(targetResolution)
                .setTargetFrameRate(Range(targetFps, targetFps))
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
                cameraSelector,
                preview,
                imageAnalysis,
            )

        usingFrontCamera = cameraSelector == CameraSelector.DEFAULT_FRONT_CAMERA
        return usingFrontCamera
    }

    private fun selectCameraSelector(
        provider: ProcessCameraProvider,
        preferFrontCamera: Boolean,
    ): CameraSelector {
        val preferredSelector = if (preferFrontCamera) CameraSelector.DEFAULT_FRONT_CAMERA else CameraSelector.DEFAULT_BACK_CAMERA
        if (hasCamera(provider, preferredSelector)) {
            return preferredSelector
        }

        val fallbackSelector = if (preferFrontCamera) CameraSelector.DEFAULT_BACK_CAMERA else CameraSelector.DEFAULT_FRONT_CAMERA
        if (hasCamera(provider, fallbackSelector)) {
            Log.w(
                TAG,
                "Requested ${if (preferFrontCamera) "front" else "back"} camera unavailable; using fallback",
            )
            return fallbackSelector
        }

        throw IllegalStateException("No camera available for preview/streaming")
    }

    private fun hasCamera(
        provider: ProcessCameraProvider,
        selector: CameraSelector,
    ): Boolean =
        runCatching {
            provider.hasCamera(selector)
        }.onFailure {
            Log.w(TAG, "Camera availability check failed", it)
        }.getOrDefault(false)

    fun stop() {
        cameraProvider?.unbindAll()
        activeCamera = null
        usingFrontCamera = false
    }

    fun release() {
        stop()
        analysisExecutor.shutdown()
    }

    companion object {
        private const val TAG = "CameraController"
        private const val SWITCH_TIMEOUT_MS = 2_000L
    }
}

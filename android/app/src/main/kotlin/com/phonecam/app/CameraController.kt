package com.phonecam.app

import android.content.Context
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.util.Log
import android.util.Range
import android.util.Size
import androidx.camera.core.Camera
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.ImageProxy
import androidx.camera.core.Preview
import androidx.camera.camera2.interop.Camera2Interop
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.core.content.ContextCompat
import androidx.camera.core.resolutionselector.ResolutionSelector
import androidx.camera.core.resolutionselector.ResolutionStrategy
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
        val exactResolution =
            ResolutionSelector.Builder()
                .setResolutionStrategy(
                    ResolutionStrategy(
                        targetResolution,
                        ResolutionStrategy.FALLBACK_RULE_NONE,
                    ),
                )
                .build()
        val fpsRange = narrowestFpsRange(useFrontCamera, targetFps)

        val previewBuilder =
            Preview.Builder()
                .setResolutionSelector(exactResolution)
        fpsRange?.let {
            Camera2Interop.Extender(previewBuilder)
                .setCaptureRequestOption(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it)
        }
        val preview =
            previewBuilder
                .build()
                .also {
                    it.setSurfaceProvider(previewView.surfaceProvider)
                }

        val analysisBuilder =
            ImageAnalysis.Builder()
                .setResolutionSelector(exactResolution)
                .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
        fpsRange?.let {
            Camera2Interop.Extender(analysisBuilder)
                .setCaptureRequestOption(CaptureRequest.CONTROL_AE_TARGET_FPS_RANGE, it)
        }
        val imageAnalysis =
            analysisBuilder
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

        val boundResolution =
            imageAnalysis.resolutionInfo?.resolution
                ?: throw IllegalStateException("CameraX did not report the bound analysis resolution")
        if (boundResolution != targetResolution) {
            provider.unbindAll()
            throw IllegalStateException(
                "CameraX bound ${boundResolution.width}x${boundResolution.height}, expected " +
                    "${targetResolution.width}x${targetResolution.height}",
            )
        }
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

    fun discoverExactCaptureProfiles(useFrontCamera: Boolean = false): Set<Pair<StreamResolution, Int>> {
        val cameraManager = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val desiredFacing =
            if (useFrontCamera) {
                CameraCharacteristics.LENS_FACING_FRONT
            } else {
                CameraCharacteristics.LENS_FACING_BACK
            }
        val cameraId =
            cameraManager.cameraIdList.firstOrNull { id ->
                cameraManager.getCameraCharacteristics(id)
                    .get(CameraCharacteristics.LENS_FACING) == desiredFacing
            } ?: return emptySet()
        val characteristics = cameraManager.getCameraCharacteristics(cameraId)
        val map =
            characteristics.get(CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP)
                ?: return emptySet()
        val sizes = map.getOutputSizes(ImageFormat.YUV_420_888)?.toSet().orEmpty()
        val fpsRanges =
            characteristics.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
                ?.toList()
                .orEmpty()
        return buildSet {
            for (resolution in StreamResolution.entries) {
                val size = Size(resolution.width, resolution.height)
                if (size !in sizes) {
                    continue
                }
                val minimumDuration =
                    map.getOutputMinFrameDuration(ImageFormat.YUV_420_888, size)
                for (fps in listOf(15, 30, 60)) {
                    if (
                        fpsRanges.any { range -> range.contains(fps) } &&
                        (minimumDuration <= 0L || minimumDuration <= 1_000_000_000L / fps)
                    ) {
                        add(resolution to fps)
                    }
                }
            }
        }
    }

    private fun narrowestFpsRange(
        useFrontCamera: Boolean,
        targetFps: Int,
    ): Range<Int>? {
        val cameraManager = context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
        val desiredFacing =
            if (useFrontCamera) {
                CameraCharacteristics.LENS_FACING_FRONT
            } else {
                CameraCharacteristics.LENS_FACING_BACK
            }
        return cameraManager.cameraIdList
            .firstOrNull { id ->
                cameraManager.getCameraCharacteristics(id)
                    .get(CameraCharacteristics.LENS_FACING) == desiredFacing
            }
            ?.let(cameraManager::getCameraCharacteristics)
            ?.get(CameraCharacteristics.CONTROL_AE_AVAILABLE_TARGET_FPS_RANGES)
            ?.filter { targetFps in it }
            ?.minWithOrNull(compareBy<Range<Int>>({ it.upper - it.lower }, { it.lower }))
    }

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

package com.mdnssknght.mycamera.fragment

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Bitmap
import android.graphics.Color
import android.graphics.ImageFormat
import android.hardware.camera2.CameraCaptureSession
import android.hardware.camera2.CameraCharacteristics
import android.hardware.camera2.CameraDevice
import android.hardware.camera2.CameraManager
import android.hardware.camera2.CaptureRequest
import android.hardware.camera2.CaptureResult
import android.hardware.camera2.DngCreator
import android.hardware.camera2.TotalCaptureResult
import android.media.Image
import android.media.ImageReader
import android.media.MediaScannerConnection
import android.os.Bundle
import android.os.Environment
import android.os.Handler
import android.os.HandlerThread
import android.util.Log
import android.util.Rational
import android.view.LayoutInflater
import android.view.Surface
import android.view.SurfaceHolder
import android.view.View
import android.view.ViewGroup
import androidx.core.graphics.createBitmap
import androidx.core.graphics.drawable.toDrawable
import androidx.exifinterface.media.ExifInterface
import androidx.fragment.app.Fragment
import androidx.lifecycle.lifecycleScope
import androidx.navigation.NavController
import androidx.navigation.findNavController
import androidx.navigation.fragment.navArgs
import com.mdnssknght.mycamera.R
import com.mdnssknght.mycamera.activity.CameraActivity
import com.mdnssknght.mycamera.databinding.FragmentCameraBinding
import com.mdnssknght.mycamera.processing.RawProcessor
import com.mdnssknght.mycamera.util.OrientationLiveData
import com.mdnssknght.mycamera.util.computeExifOrientation
import com.mdnssknght.mycamera.util.getPreviewOutputSize
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.suspendCancellableCoroutine
import java.io.Closeable
import java.io.File
import java.io.FileOutputStream
import java.io.IOException
import java.nio.ByteBuffer
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.TimeoutException
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException
import kotlin.coroutines.suspendCoroutine

class CameraFragment : Fragment() {
    /** Android ViewBinding. */
    private var _fragmentCameraBinding: FragmentCameraBinding? = null

    private val fragmentCameraBinding get() = _fragmentCameraBinding!!

    /** AndroidX navigation arguments. */
    private val args: CameraFragmentArgs by navArgs()

    /** Host's navigation controller. */
    private val navController: NavController by lazy {
        requireActivity().findNavController(R.id.fragment_container)
    }

    /** Detects characterizes and connects to a new [CameraDevice] (used for all camera operations). */
    private val cameraManager: CameraManager by lazy {
        val context = requireContext().applicationContext
        context.getSystemService(Context.CAMERA_SERVICE) as CameraManager
    }

    /** The [CameraCharacteristics] that corresponds to the provided camera ID. */
    private val characteristics: CameraCharacteristics by lazy {
        cameraManager.getCameraCharacteristics(args.cameraId)
    }

    /** Readers used as buffers for camera still shots. */
    private lateinit var imageReader: ImageReader

    /** [HandlerThread] where all camera operations run. */
    private val cameraThread = HandlerThread("CameraThread").apply { start() }

    /** The [Handler] that corresponds to [cameraThread]. */
    private val cameraHandler = Handler(cameraThread.looper)

    /** Performs recording animation of flashing screen. */
    private val animationTask: Runnable by lazy {
        Runnable {
            // Flash white animation.
            fragmentCameraBinding.overlay.background = Color.argb(150, 255, 255, 255).toDrawable()
            // Wait for ANIMATION_FAST_MILLIS.
            fragmentCameraBinding.overlay.postDelayed({
                // Remove white flash animation.
                fragmentCameraBinding.overlay.background = null
            }, CameraActivity.ANIMATION_FAST_MILLIS)
        }
    }

    /** [HandlerThread] where all buffer reading operations run. */
    private val imageReaderThread = HandlerThread("imageReaderThread").apply { start() }

    /** The [Handler] that corresponds to [imageReaderThread]. */
    private val imageReaderHandler = Handler(imageReaderThread.looper)

    /** The [CameraDevice] that will be opened in this fragment. */
    private lateinit var camera: CameraDevice

    /** Internal reference to the ongoing [CameraCaptureSession] configured with our parameters. */
    private lateinit var session: CameraCaptureSession

    /** Live data listener for changes in the device orientation relative to the camera. */
    private lateinit var relativeOrientation: OrientationLiveData

    override fun onCreateView(
        inflater: LayoutInflater,
        container: ViewGroup?,
        savedInstanceState: Bundle?
    ): View {
        _fragmentCameraBinding = FragmentCameraBinding.inflate(inflater, container, false)
        return fragmentCameraBinding.root
    }

    override fun onViewCreated(view: View, savedInstanceState: Bundle?) {
        super.onViewCreated(view, savedInstanceState)

        fragmentCameraBinding.captureButton.setOnApplyWindowInsetsListener { v, insets ->
            v.translationX = (-insets.systemWindowInsetRight).toFloat()
            v.translationY = (-insets.systemWindowInsetBottom).toFloat()
            insets.consumeSystemWindowInsets()
        }

        fragmentCameraBinding.viewfinder.holder.addCallback(object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) {
                // Selects appropriate preview size and configures viewfinder.
                val previewSize = getPreviewOutputSize(
                    fragmentCameraBinding.viewfinder.display,
                    characteristics,
                    SurfaceHolder::class.java
                )

                Log.d(
                    TAG,
                    "Viewfinder size: ${fragmentCameraBinding.viewfinder.width}x${fragmentCameraBinding.viewfinder.height}"
                )
                Log.d(TAG, "Selected preview size: $previewSize")

                fragmentCameraBinding.viewfinder.setAspectRatio(
                    previewSize.width,
                    previewSize.height
                )

                // To ensure that size is set, initialize camera in the view's thread.
                view.post { initializeCamera() }
            }

            override fun surfaceChanged(
                holder: SurfaceHolder,
                format: Int,
                width: Int,
                height: Int
            ) = Unit

            override fun surfaceDestroyed(holder: SurfaceHolder) = Unit
        })

        // Used to rotate the output media to match device orientation.
        relativeOrientation = OrientationLiveData(requireContext(), characteristics).apply {
            observe(viewLifecycleOwner) { orientation ->
                Log.d(TAG, "Orientation changed: $orientation")
            }
        }
    }

    /**
     * Begin all camera operations in a coroutine in the main thread. This function:
     * - Opens the camera.
     * - Configures the camera session.
     * - Starts the preview by dispatching a repeating capture request.
     * - Sets up the still image capture listeners.
     */
    private fun initializeCamera() = lifecycleScope.launch(Dispatchers.Main) {
        // Open selected camera.
        camera = openCamera(cameraManager, args.cameraId, cameraHandler)

        val size = characteristics.get(
            CameraCharacteristics.SCALER_STREAM_CONFIGURATION_MAP
        )!!
            .getOutputSizes(args.pixelFormat).maxByOrNull { it.width * it.height }!!

        // Initialize an image reader which will be used to capture still photos.
        imageReader =
            ImageReader.newInstance(size.width, size.height, args.pixelFormat, IMAGE_BUFFER_SIZE)

        // Creates a list of surfaces where the camera will output frames.
        val targets = listOf(fragmentCameraBinding.viewfinder.holder.surface, imageReader.surface)

        // Starts a capture session using our open camera and list of surfaces where the frames will go.
        session = createCaptureSession(camera, targets, cameraHandler)

        val captureRequest = camera.createCaptureRequest(CameraDevice.TEMPLATE_PREVIEW).apply {
            addTarget(fragmentCameraBinding.viewfinder.holder.surface)
        }

        // This will keep sending the capture request as frequently as possible until the session
        // is torn down or session.stopRepeating() is called.
        session.setRepeatingRequest(captureRequest.build(), null, cameraHandler)

        // Listen to the capture button.
        fragmentCameraBinding.captureButton.setOnClickListener { it ->

            // Disable click listener to prevent multiple requests simultaneously in flight.
            it.isEnabled = false

            // Perform heavy I/O operations in a different scope.
            lifecycleScope.launch(Dispatchers.IO) {
                takePhoto().use { result ->
                    Log.d(TAG, "Result received: $result")

                    // Save the output result to disk.
                    val output = saveResult(result)
                    Log.d(TAG, "Image saved ${output.absolutePath}")

                    when (output.extension) {

                        // If the result is a JPEG file, update EXIF metadata with orientation info.
                        "jpg" -> {
                            ExifInterface(output.absolutePath).let { exif ->
                                exif.setAttribute(
                                    ExifInterface.TAG_ORIENTATION,
                                    result.orientation.toString()
                                )
                                exif.saveAttributes()
                                Log.d(TAG, "EXIF metadata saved: ${output.absolutePath}")
                            }
                        }

                        // If the result is a RAW file, then pass its data for further processing.
                        "dng" -> {
                            val outputBuffer: ByteBuffer
                            val width: Int
                            val height: Int

                            result.image.let { it ->
                                width = it.planes[0].rowStride / it.planes[0].pixelStride
                                height = it.height

                                val outputBytes = ByteArray(width * height * 4)

                                val colorFilterArrangement = characteristics.get(
                                    CameraCharacteristics.SENSOR_INFO_COLOR_FILTER_ARRANGEMENT
                                )!!

                                val colorGains = FloatArray(4)
                                result.metadata.get(CaptureResult.COLOR_CORRECTION_GAINS)!!
                                    .copyTo(colorGains, 0)

                                val whiteLevel =
                                    characteristics.get(CameraCharacteristics.SENSOR_INFO_WHITE_LEVEL)!!

                                val blackLevel = IntArray(4)
                                characteristics.get(CameraCharacteristics.SENSOR_BLACK_LEVEL_PATTERN)!!
                                    .copyTo(blackLevel, 0)

                                val forwardMatrix1 = FloatArray(9)
                                val forwardMatrix2 = FloatArray(9)

                                val rationalDestination = arrayOfNulls<Rational>(9)

                                characteristics.get(CameraCharacteristics.SENSOR_FORWARD_MATRIX1)!!
                                    .copyElements(rationalDestination, 0)
                                rationalDestination.forEachIndexed { index, rational ->
                                    forwardMatrix1[index] = rational!!.toFloat()
                                }

                                characteristics.get(CameraCharacteristics.SENSOR_FORWARD_MATRIX2)!!
                                    .copyElements(rationalDestination, 0)
                                rationalDestination.forEachIndexed { index, rational ->
                                    forwardMatrix2[index] = rational!!.toFloat()
                                }

                                RawProcessor.process(
                                    width,
                                    height,
                                    it.planes[0].buffer,
                                    outputBytes,
                                    colorFilterArrangement,
                                    whiteLevel,
                                    blackLevel,
                                    colorGains,
                                    forwardMatrix1,
                                    forwardMatrix2
                                )

                                outputBuffer = ByteBuffer.wrap(outputBytes)
                            }

                            // Hacky, I know
                            try {
                                val bitmap = createBitmap(width, height)
                                    .apply { copyPixelsFromBuffer(outputBuffer) }

                                val file = createFile("jpg")
                                FileOutputStream(file).use { it ->
                                    bitmap.compress(
                                        Bitmap.CompressFormat.JPEG,
                                        100, it
                                    )
                                }

                                ExifInterface(file.absolutePath).let { exif ->
                                    exif.setAttribute(
                                        ExifInterface.TAG_ORIENTATION,
                                        result.orientation.toString()
                                    )
                                    exif.saveAttributes()
                                    Log.d(TAG, "EXIF metadata saved: ${output.absolutePath}")
                                }

                                // Even more hacky. Yes, I know
                                MediaScannerConnection.scanFile(
                                    context,
                                    arrayOf(file.absolutePath),
                                    null,
                                    null
                                )

                            } catch (e: Exception) {
                                Log.e(TAG, "Error saving processed JPEG", e)
                            }
                        }

                        else -> {}
                    }

                    MediaScannerConnection.scanFile(
                        context,
                        arrayOf(output.absolutePath),
                        null,
                        null
                    )

                    // Display the photo taken to the user.
                    lifecycleScope.launch(Dispatchers.Main) {
                        navController.navigate(
                            CameraFragmentDirections
                                .actionCameraToJpegViewer(output.absolutePath)
                                .setOrientation(result.orientation)
                                .setDepth(result.format == ImageFormat.DEPTH_JPEG)
                        )
                    }
                }

                // Re-enable click listener after the photo was taken.
                it.post { it.isEnabled = true }
            }
        }
    }


    /** Opens the camera and returns the opened device (as the result of the suspend coroutine). */
    @SuppressLint("MissingPermission")
    private suspend fun openCamera(
        manager: CameraManager,
        cameraId: String,
        handler: Handler? = null
    ): CameraDevice = suspendCancellableCoroutine { cont ->
        manager.openCamera(cameraId, object : CameraDevice.StateCallback() {
            override fun onOpened(camera: CameraDevice) = cont.resume(camera)

            override fun onDisconnected(camera: CameraDevice) {
                Log.w(TAG, "Camera $cameraId has been disconnected.")
                requireActivity().finish()
            }

            override fun onError(camera: CameraDevice, error: Int) {
                val msg = when (error) {
                    ERROR_CAMERA_DEVICE -> "Fatal (device)"
                    ERROR_CAMERA_DISABLED -> "Device policy"
                    ERROR_CAMERA_IN_USE -> "Camera in use"
                    ERROR_CAMERA_SERVICE -> "Fatal (service)"
                    ERROR_MAX_CAMERAS_IN_USE -> "Maximum cameras in use"
                    else -> "Unknown"
                }

                val exc = RuntimeException("Camera $cameraId error: ($error) $msg")
                Log.e(TAG, exc.message, exc)

                if (cont.isActive) cont.resumeWithException(exc)
            }

        }, handler)
    }

    /**
     * Starts a [CameraCaptureSession] and returns the configured session (as the result of the
     * suspend coroutine.
     */
    private suspend fun createCaptureSession(
        device: CameraDevice,
        targets: List<Surface>,
        handler: Handler? = null
    ): CameraCaptureSession = suspendCoroutine { cont ->

        // Create a capture session using the predefined targets; this also involves defining the
        // session state callback to be notified of when the session is ready.
        device.createCaptureSession(targets, object : CameraCaptureSession.StateCallback() {

            override fun onConfigured(session: CameraCaptureSession) = cont.resume(session)

            override fun onConfigureFailed(session: CameraCaptureSession) {
                val exc = RuntimeException("Camera ${device.id} session configuration failed")

                Log.e(TAG, exc.message, exc)

                cont.resumeWithException(exc)
            }

        }, handler)
    }

    /**
     * Helper function used to capture a still image using the [CameraDevice.TEMPLATE_STILL_CAPTURE]
     * template. It performs synchronization between the [CaptureResult] and the [Image] resulting
     * from the single capture, and outputs a [CombinedCaptureResult] object.
     */
    private suspend fun takePhoto(): CombinedCaptureResult = suspendCoroutine { cont ->

        // Flush any images left in the image reader.
        while (imageReader.acquireNextImage() != null) {
        }

        // Start a new image queue.
        val imageQueue = ArrayBlockingQueue<Image>(IMAGE_BUFFER_SIZE)

        imageReader.setOnImageAvailableListener({ reader ->
            val image = reader.acquireNextImage()
            Log.d(TAG, "Image available in queue: ${image.timestamp}")
            imageQueue.add(image)
        }, imageReaderHandler)

        val captureRequest =
            session.device.createCaptureRequest(CameraDevice.TEMPLATE_STILL_CAPTURE)
                .apply { addTarget(imageReader.surface) }

        session.capture(captureRequest.build(), object : CameraCaptureSession.CaptureCallback() {

            override fun onCaptureStarted(
                session: CameraCaptureSession,
                request: CaptureRequest,
                timestamp: Long,
                frameNumber: Long
            ) {
                super.onCaptureStarted(session, request, timestamp, frameNumber)

                fragmentCameraBinding.viewfinder.post(animationTask)
            }

            override fun onCaptureCompleted(
                session: CameraCaptureSession,
                request: CaptureRequest,
                result: TotalCaptureResult
            ) {
                super.onCaptureCompleted(session, request, result)

                val resultTimestamp = result.get(CaptureResult.SENSOR_TIMESTAMP)
                Log.d(TAG, "Capture result received: $resultTimestamp")

                // Set a timeout in case the image captured is dropped from the pipeline.
                val exc = TimeoutException("Image dequeue took too long")
                val timeoutRunnable = Runnable { cont.resumeWithException(exc) }
                imageReaderHandler.postDelayed(timeoutRunnable, IMAGE_CAPTURE_TIMEOUT_MILLIS)

                // Loop in the coroutine's context until an image with matching timestamp comes.
                // We need to launch the coroutine context again because the callback is done in
                // the handler provided to the `capture` method, not in our coroutine context.
                lifecycleScope.launch(cont.context) {
                    while (true) {

                        // Dequeue images while timestamps don't match.
                        val image = imageQueue.take()
                        if (image.format != ImageFormat.DEPTH_JPEG &&
                            image.timestamp != resultTimestamp
                        ) continue
                        Log.d(TAG, "Matching image dequeued: ${image.timestamp}")

                        // Unset the image reader listener.
                        imageReaderHandler.removeCallbacks(timeoutRunnable)
                        imageReader.setOnImageAvailableListener(null, null)

                        // Clear the queue of images, if there are any left.
                        while (imageQueue.isNotEmpty()) imageQueue.take().close()

                        // Compute EXIF orientation metadata.
                        val rotation = relativeOrientation.value ?: 0
                        val mirrored = characteristics.get(CameraCharacteristics.LENS_FACING) ==
                                CameraCharacteristics.LENS_FACING_FRONT
                        val exifOrientation = computeExifOrientation(rotation, mirrored)

                        // Build the result and resume progress.
                        cont.resume(
                            CombinedCaptureResult(
                                image, result, exifOrientation, imageReader.imageFormat
                            )
                        )

                        // There is no need to break out of the loop, this coroutine will suspend.
                    }
                }
            }

        }, cameraHandler)
    }

    /** Helper function used to save a [CombinedCaptureResult] into a [File]. */
    private suspend fun saveResult(result: CombinedCaptureResult): File = suspendCoroutine { cont ->
        when (result.format) {

            // When the format is JPEG or DEPTH JPEG we can simply save the bytes as-is.
            ImageFormat.JPEG, ImageFormat.DEPTH_JPEG -> {
                val buffer = result.image.planes[0].buffer
                val bytes = ByteArray(buffer.remaining()).apply { buffer.get(this) }
                try {
                    val output = createFile("jpg")
                    FileOutputStream(output).use { it.write(bytes) }
                    cont.resume(output)
                } catch (exc: IOException) {
                    Log.e(TAG, "Unable to write JPEG image to file", exc)
                    cont.resumeWithException(exc)
                }
            }

            // When the format is RAW we use the DngCreator utility class.
            ImageFormat.RAW_SENSOR -> {
                val dngCreator = DngCreator(characteristics, result.metadata)
                    .setOrientation(result.orientation)
                try {
                    val output = createFile("dng")
                    FileOutputStream(output).use { dngCreator.writeImage(it, result.image) }
                    cont.resume(output)
                } catch (exc: IOException) {
                    Log.e(TAG, "Unable to write DNG image to file", exc)
                    cont.resumeWithException(exc)
                }
            }

            // No other formats are supported.
            else -> {
                val exc = RuntimeException("Unknown image format: ${result.image.format}")
                Log.e(TAG, exc.message, exc)
                cont.resumeWithException(exc)
            }
        }
    }

    override fun onStop() {
        super.onStop()

        try {
            camera.close()
        } catch (exc: Throwable) {
            Log.e(TAG, "Error closing camera", exc)
        }
    }

    override fun onDestroy() {
        super.onDestroy()

        cameraThread.quitSafely()
        imageReaderThread.quitSafely()
    }

    override fun onDestroyView() {
        _fragmentCameraBinding = null

        super.onDestroyView()
    }

    companion object {

        private val TAG = CameraFragment::class.java.simpleName

        /** Maximum number of images that will be held in the reader's buffer. */
        private const val IMAGE_BUFFER_SIZE: Int = 3

        /** Maximum time allowed to wait for the result of an image capture. */
        private const val IMAGE_CAPTURE_TIMEOUT_MILLIS: Long = 5000

        /** Helper data class used to hold capture metadata with their associated image. */
        data class CombinedCaptureResult(
            val image: Image,
            val metadata: CaptureResult,
            val orientation: Int,
            val format: Int
        ) : Closeable {

            override fun close() = image.close()
        }

        /**
         * Create a [File] named a using formatted timestamp with the current date and time.
         *
         * @return [File] created.
         */
        private fun createFile(extension: String): File {
            val path = File(
                Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_DCIM),
                "MyCamera/"
            )
            path.mkdirs()
            val sdf = SimpleDateFormat("yyyy_MM_dd_HH_mm_ss_SSS", Locale.US)
            return File(path, "IMG_${sdf.format(Date())}.$extension")
        }
    }
}
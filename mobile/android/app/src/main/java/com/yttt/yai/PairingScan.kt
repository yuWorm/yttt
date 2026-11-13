package com.yttt.yai

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Bitmap
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.camera.core.CameraSelector
import androidx.camera.core.ImageAnalysis
import androidx.camera.core.Preview
import androidx.camera.lifecycle.ProcessCameraProvider
import androidx.camera.view.PreviewView
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.content.ContextCompat
import androidx.lifecycle.compose.LocalLifecycleOwner
import com.google.zxing.BinaryBitmap
import com.google.zxing.DecodeHintType
import com.google.zxing.PlanarYUVLuminanceSource
import com.google.zxing.ReaderException
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicBoolean

private const val PAIRING_PREFIX = "YTTT-PAIR:"

/** The pairing code in a QR code in a luminance (Y) plane, if any. */
fun decodePairingCode(luminance: ByteArray, width: Int, height: Int): String? {
    val source = PlanarYUVLuminanceSource(luminance, width, height, 0, 0, width, height, false)
    return try {
        QRCodeReader()
            .decode(BinaryBitmap(HybridBinarizer(source)), mapOf(DecodeHintType.TRY_HARDER to true))
            .text
            .takeIf { it.startsWith(PAIRING_PREFIX) }
    } catch (_: ReaderException) {
        null
    }
}

/** A picture's luminance plane, for the same decoder as the camera's frames. */
fun Bitmap.luminance(): ByteArray {
    val pixels = IntArray(width * height)
    getPixels(pixels, 0, width, 0, 0, width, height)
    return ByteArray(pixels.size) { index ->
        val pixel = pixels[index]
        val r = (pixel shr 16) and 0xff
        val g = (pixel shr 8) and 0xff
        val b = pixel and 0xff
        ((r * 299 + g * 587 + b * 114) / 1000).toByte()
    }
}

/** The camera, looking for a pairing QR code; hands over the first it reads. */
@Composable
fun PairingScanner(onCode: (String) -> Unit, onClose: () -> Unit) {
    val context = LocalContext.current
    val lifecycle = LocalLifecycleOwner.current
    var allowed by remember {
        mutableStateOf(ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED)
    }
    val ask = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { allowed = it }
    LaunchedEffect(Unit) { if (!allowed) ask.launch(Manifest.permission.CAMERA) }
    val analysis = remember { Executors.newSingleThreadExecutor() }
    DisposableEffect(Unit) { onDispose { analysis.shutdown() } }

    Box(Modifier.fillMaxSize()) {
        if (allowed) {
            AndroidView(
                factory = { viewContext ->
                    val preview = PreviewView(viewContext)
                    val found = AtomicBoolean(false)
                    val providerFuture = ProcessCameraProvider.getInstance(viewContext)
                    providerFuture.addListener({
                        val provider = providerFuture.get()
                        val usecasePreview = Preview.Builder().build().also { it.surfaceProvider = preview.surfaceProvider }
                        val frames = ImageAnalysis.Builder()
                            .setBackpressureStrategy(ImageAnalysis.STRATEGY_KEEP_ONLY_LATEST)
                            .build()
                        frames.setAnalyzer(analysis) { image ->
                            image.use {
                                if (found.get()) return@use
                                val plane = it.planes[0]
                                val width = it.width
                                val height = it.height
                                val row = plane.rowStride
                                val buffer = plane.buffer
                                // Rows can be padded: copy each row's pixels only.
                                val luminance = ByteArray(width * height)
                                for (y in 0 until height) {
                                    buffer.position(y * row)
                                    buffer.get(luminance, y * width, width)
                                }
                                decodePairingCode(luminance, width, height)?.let { code ->
                                    if (found.compareAndSet(false, true)) {
                                        ContextCompat.getMainExecutor(viewContext).execute { onCode(code) }
                                    }
                                }
                            }
                        }
                        provider.unbindAll()
                        provider.bindToLifecycle(lifecycle, CameraSelector.DEFAULT_BACK_CAMERA, usecasePreview, frames)
                    }, ContextCompat.getMainExecutor(viewContext))
                    preview
                },
                modifier = Modifier.fillMaxSize(),
            )
            Text("对准电脑上的配对二维码", color = Color.White,
                modifier = Modifier.align(Alignment.TopCenter).padding(24.dp))
        } else {
            Text("扫码需要相机权限", Modifier.align(Alignment.Center))
        }
        TextButton(onClick = onClose, modifier = Modifier.align(Alignment.BottomCenter).padding(24.dp)) {
            Text("取消")
        }
    }
}

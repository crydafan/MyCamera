package com.mdnssknght.mycamera.processing

import java.nio.ByteBuffer

object RawProcessor {
    private var pointerHandle: Long = 0

    init {
        pointerHandle = NativeRawProcessor.nativeInit()
    }

    fun init() {
        // Because this is an object we want the pointer to the handle to be initialized
        // only once.
    }

    fun fini() {
        NativeRawProcessor.nativeFini(pointerHandle)
    }

    fun process(
        width: Int,
        height: Int,
        data: ByteBuffer,
        out: ByteArray,
        colorFilterArrangement: Int,
        whiteLevel: Int,
        blackLevel: IntArray,
        neutralPoint: FloatArray,
        colorGains: FloatArray,
        forwardMatrix1: FloatArray,
        forwardMatrix2: FloatArray,
    ) {
        NativeRawProcessor.nativeProcess(
            pointerHandle,
            width,
            height,
            data,
            out,
            colorFilterArrangement,
            whiteLevel,
            blackLevel,
            neutralPoint,
            colorGains,
            forwardMatrix1,
            forwardMatrix2
        )
    }
}
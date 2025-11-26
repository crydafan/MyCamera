package com.mdnssknght.mycamera.processing

import java.nio.ByteBuffer

class NativeRawProcessor {

    companion object {
        init {
            System.loadLibrary("raw_processor")
        }

        external fun nativeInit(): Long

        external fun nativeFini(handle: Long)

        external fun nativeProcess(
            handle: Long,
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
        )
    }
}
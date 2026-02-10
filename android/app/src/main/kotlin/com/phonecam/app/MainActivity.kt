package com.phonecam.app

import android.os.Bundle
import android.util.Log
import android.view.Gravity
import android.widget.FrameLayout
import android.widget.TextView
import androidx.appcompat.app.AppCompatActivity
import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer

class MainActivity : AppCompatActivity() {

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        runCatching {
            RustBridge.ffiTestMessage()
        }.onSuccess {
            Log.i(TAG, "Rust FFI verification message: $it")
        }.onFailure {
            Log.e(TAG, "Rust FFI verification failed", it)
        }

        val root =
            FrameLayout(this).apply {
                layoutParams =
                    FrameLayout.LayoutParams(
                        FrameLayout.LayoutParams.MATCH_PARENT,
                        FrameLayout.LayoutParams.MATCH_PARENT,
                    )
            }

        val placeholder =
            TextView(this).apply {
                text = "PhoneCam Android - Placeholder"
                textSize = 24f
                gravity = Gravity.CENTER
            }

        root.addView(
            placeholder,
            FrameLayout.LayoutParams(
                FrameLayout.LayoutParams.MATCH_PARENT,
                FrameLayout.LayoutParams.MATCH_PARENT,
            ),
        )

        setContentView(root)
    }

    private interface PhoneCamRustLib : Library {
        fun phonecam_ffi_test_message(): Pointer?

        fun phonecam_string_free(ptr: Pointer?)
    }

    private object RustBridge {
        private val lib: PhoneCamRustLib =
            Native.load("phonecam_mobile_core", PhoneCamRustLib::class.java)

        fun ffiTestMessage(): String {
            val raw = lib.phonecam_ffi_test_message()
                ?: error("phonecam_ffi_test_message returned null")

            return try {
                raw.getString(0)
            } finally {
                lib.phonecam_string_free(raw)
            }
        }
    }

    companion object {
        private const val TAG = "PhoneCamAndroid"
    }
}

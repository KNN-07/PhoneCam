package com.phonecam.app

import com.sun.jna.Library
import com.sun.jna.Native
import com.sun.jna.Pointer
import org.junit.Assert.assertEquals
import org.junit.Test

class RustBridgeTest {
    private interface PhoneCamRustLib : Library {
        fun phonecam_ffi_test_message(): Pointer?

        fun phonecam_string_free(ptr: Pointer?)
    }

    @Test
    fun kotlin_can_call_rust_ffi_test_message() {
        val lib = Native.load("phonecam_mobile_core", PhoneCamRustLib::class.java)

        val raw = lib.phonecam_ffi_test_message()
            ?: error("phonecam_ffi_test_message returned null")

        val message =
            try {
                raw.getString(0)
            } finally {
                lib.phonecam_string_free(raw)
            }

        assertEquals("phonecam-mobile-core ffi ok", message)
    }
}

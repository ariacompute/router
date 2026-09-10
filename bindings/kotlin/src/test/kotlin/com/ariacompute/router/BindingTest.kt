package com.ariacompute.router

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFails
import kotlin.test.assertTrue

class BindingTest {
    private fun ffiReady(): Boolean {
        val lib = System.getenv("ARIA_ROUTER_FFI_LIB")
        val cfg = System.getenv("ARIA_ROUTER_CONFIG")
        return !lib.isNullOrEmpty() && !cfg.isNullOrEmpty() && java.io.File(lib).isFile
    }

    @Test
    fun initModelsComplete() {
        if (!ffiReady()) return
        val cfg = System.getenv("ARIA_ROUTER_CONFIG")!!
        Router().init(cfg).use { r ->
            val models = r.models()
            assertTrue(models.toString().contains("semantic-auto"), models.toString())
            val out = r.complete(
                listOf(mapOf("role" to "user", "content" to "hi")),
                mapOf("model" to "ariacompute/semantic-auto"),
            )
            assertTrue(out.toString().contains("hello-from-router"), out.toString())
            assertEquals("semantic", r.lastRoute()["layer"])
        }
    }

    @Test
    fun initMissingPath() {
        if (!ffiReady()) return
        assertFails { Router().init("/no/such.yaml") }
    }

    @Test
    fun connectWithoutServer() {
        if (!ffiReady()) return
        Router().connect("http://127.0.0.1:9").use { }
    }
}

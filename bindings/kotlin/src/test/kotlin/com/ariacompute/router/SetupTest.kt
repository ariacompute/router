package com.ariacompute.router

import kotlin.test.Test
import kotlin.test.assertEquals

class SetupTest {
    @Test
    fun memoryOnly() {
        val st = applySetup(SetupConfig(), token = "t")
        assertEquals("t", st.token)
    }

    @Test
    fun routerSetupMemory() {
        val r = Router()
        r.setup(baseUrl = "http://127.0.0.1:8899", token = "t")
        assertEquals("t", r.setupStatus().token)
        r.setupClear()
        assertEquals("", r.setupStatus().token)
    }
}

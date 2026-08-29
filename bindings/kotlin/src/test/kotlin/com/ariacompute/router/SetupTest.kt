package com.ariacompute.router

import kotlin.test.Test
import kotlin.test.assertEquals

class SetupTest {
    @Test
    fun memoryOnly() {
        val st = applySetup(SetupConfig(), token = "t")
        assertEquals("t", st.token)
    }
}

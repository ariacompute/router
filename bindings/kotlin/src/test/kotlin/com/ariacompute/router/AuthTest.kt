package com.ariacompute.router

import kotlin.test.Test
import kotlin.test.assertEquals

class AuthTest {
    @Test
    fun memoryOnly() {
        val st = applyAuth(AuthConfig(), token = "t")
        assertEquals("t", st.token)
    }
}

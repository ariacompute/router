package com.ariacompute.router

data class SetupConfig(
    var baseUrl: String = "",
    var token: String = "",
)

fun applySetup(existing: SetupConfig, baseUrl: String? = null, token: String? = null): SetupConfig {
    val out = existing.copy()
    if (baseUrl != null) out.baseUrl = baseUrl
    if (token != null) out.token = token
    return out
}

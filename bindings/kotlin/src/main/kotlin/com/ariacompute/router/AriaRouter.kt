package com.ariacompute.router

data class AuthConfig(
    var baseUrl: String = "",
    var token: String = "",
)

fun applyAuth(existing: AuthConfig, baseUrl: String? = null, token: String? = null): AuthConfig {
    val out = existing.copy()
    if (baseUrl != null) out.baseUrl = baseUrl
    if (token != null) out.token = token
    return out
}

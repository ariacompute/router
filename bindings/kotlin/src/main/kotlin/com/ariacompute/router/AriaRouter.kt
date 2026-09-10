package com.ariacompute.router

import com.sun.jna.Library
import com.sun.jna.Memory
import com.sun.jna.Native
import com.sun.jna.Pointer
import java.io.File
import org.json.JSONArray
import org.json.JSONObject

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

/** Kotlin/JVM binding over libaria-router_ffi via JNA. Setup is in-memory only. */
class Router : AutoCloseable {
    private var auth = SetupConfig()
    private var lib: Lib? = null
    private var handle: Pointer? = null

    interface Lib : Library {
        fun aria_router_init(path: String?): Pointer?
        fun aria_router_connect(url: String): Pointer?
        fun aria_router_destroy(h: Pointer)
        fun aria_router_setup(h: Pointer, baseUrl: String?, token: String?)
        fun aria_router_complete(
            h: Pointer,
            messages: String,
            options: String,
            out: Pointer,
            outLen: Int,
        ): Int

        fun aria_router_models(h: Pointer, out: Pointer, outLen: Int): Int
        fun aria_router_last_route(h: Pointer, out: Pointer, outLen: Int): Int
        fun aria_router_last_error(): String?
    }

    fun setup(baseUrl: String? = null, token: String? = null): Router {
        auth = applySetup(auth, baseUrl = baseUrl, token = token)
        val h = handle
        if (h != null) {
            lib?.aria_router_setup(h, baseUrl, token)
        }
        return this
    }

    fun setupStatus(): SetupConfig = auth.copy()

    fun setupClear(): Router {
        auth = SetupConfig()
        val h = handle
        if (h != null) {
            lib?.aria_router_setup(h, "", "")
        }
        return this
    }

    private fun ensure() {
        if (lib != null) return
        lib = loadLib()
    }

    private fun err(fallback: String): String {
        val e = lib?.aria_router_last_error()
        return if (e.isNullOrEmpty()) fallback else e
    }

    private fun syncAuthIfSet() {
        val h = handle ?: return
        if (auth.baseUrl.isNotEmpty() || auth.token.isNotEmpty()) {
            lib?.aria_router_setup(
                h,
                auth.baseUrl.ifEmpty { null },
                auth.token.ifEmpty { null },
            )
        }
    }

    fun init(configPath: String? = null): Router {
        ensure()
        close()
        val pathArg =
            if (configPath.isNullOrBlank()) null
            else expandHome(configPath.trim())
        val h = lib!!.aria_router_init(pathArg)
            ?: throw IllegalStateException(err("init failed"))
        handle = h
        syncAuthIfSet()
        return this
    }

    fun connect(baseUrl: String): Router {
        ensure()
        close()
        val h = lib!!.aria_router_connect(baseUrl)
            ?: throw IllegalStateException(err("connect failed"))
        handle = h
        syncAuthIfSet()
        return this
    }

    override fun close() {
        val h = handle
        val l = lib
        if (h != null && l != null) {
            l.aria_router_destroy(h)
        }
        handle = null
    }

    fun destroy() = close()

    fun complete(messages: Any, options: Any? = null): Map<String, Any?> {
        val h = handle ?: throw IllegalStateException("router not initialized")
        val msgJson = toJson(messages)
        val optJson = toJson(options ?: emptyMap<String, Any>())
        val buf = Memory(256L * 1024L)
        buf.clear()
        val rc = lib!!.aria_router_complete(h, msgJson, optJson, buf, buf.size().toInt())
        if (rc != 0) throw IllegalStateException(err("complete failed"))
        return parseObject(buf.getString(0))
    }

    fun models(): Map<String, Any?> {
        val h = handle ?: throw IllegalStateException("router not initialized")
        val buf = Memory(64L * 1024L)
        buf.clear()
        val rc = lib!!.aria_router_models(h, buf, buf.size().toInt())
        if (rc != 0) throw IllegalStateException(err("models failed"))
        return parseObject(buf.getString(0))
    }

    fun lastRoute(): Map<String, Any?> {
        val h = handle ?: return emptyMap()
        val buf = Memory(64L * 1024L)
        buf.clear()
        val rc = lib!!.aria_router_last_route(h, buf, buf.size().toInt())
        if (rc != 0) return emptyMap()
        return try {
            parseObject(buf.getString(0).ifEmpty { "{}" })
        } catch (_: Exception) {
            emptyMap()
        }
    }

    companion object {
        private fun ariaHome(): String {
            val override = System.getenv("ARIA_COMPUTE_HOME")
            if (!override.isNullOrEmpty()) return override
            return System.getProperty("user.home") + File.separator + ".ariacompute"
        }

        private fun ffiLibNames(): List<String> {
            val os = (System.getProperty("os.name") ?: "").lowercase()
            return when {
                os.contains("win") -> listOf("aria-router_ffi.dll", "aria_router_ffi.dll")
                os.contains("mac") || os.contains("darwin") ->
                    listOf("libaria-router_ffi.dylib", "libaria_router_ffi.dylib")
                else -> listOf("libaria-router_ffi.so", "libaria_router_ffi.so")
            }
        }

        private fun firstExisting(dir: File): File? {
            for (name in ffiLibNames()) {
                val f = File(dir, name)
                if (f.isFile) return f
            }
            return null
        }

        internal fun loadLib(): Lib {
            val env = System.getenv("ARIA_ROUTER_FFI_LIB")
            if (!env.isNullOrEmpty()) {
                val f = File(env)
                if (f.isFile) {
                    return Native.load(f.absolutePath, Lib::class.java)
                }
            }
            val cached = firstExisting(File(ariaHome(), "lib"))
            if (cached != null) {
                return Native.load(cached.absolutePath, Lib::class.java)
            }
            // Last resort: hyphenated name on library path.
            return Native.load("aria-router_ffi", Lib::class.java)
        }

        private fun expandHome(path: String): String {
            if (path == "~") return System.getProperty("user.home")
            if (path.startsWith("~/")) {
                return System.getProperty("user.home") + File.separator + path.substring(2)
            }
            return path
        }

        private fun toJson(value: Any): String =
            when (value) {
                is String -> value
                is JSONObject -> value.toString()
                is JSONArray -> value.toString()
                is Map<*, *> -> JSONObject(value).toString()
                is List<*> -> JSONArray(value).toString()
                else -> JSONObject.wrap(value)?.toString() ?: "{}"
            }

        private fun parseObject(raw: String): Map<String, Any?> {
            if (raw.isBlank()) return emptyMap()
            val obj = JSONObject(raw)
            val out = linkedMapOf<String, Any?>()
            for (key in obj.keys()) {
                out[key] = unwrap(obj.get(key))
            }
            return out
        }

        private fun unwrap(v: Any?): Any? =
            when (v) {
                null, JSONObject.NULL -> null
                is JSONObject -> {
                    val m = linkedMapOf<String, Any?>()
                    for (k in v.keys()) m[k] = unwrap(v.get(k))
                    m
                }
                is JSONArray -> (0 until v.length()).map { unwrap(v.get(it)) }
                else -> v
            }
    }
}

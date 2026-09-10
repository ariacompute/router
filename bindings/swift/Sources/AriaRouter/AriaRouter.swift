#if canImport(Darwin)
import Darwin
#else
import Glibc
#endif
import Foundation

public struct AriaRouterAuth {
    public var baseUrl: String = ""
    public var token: String = ""
    public init() {}
}

public func applyRouterAuth(_ existing: AriaRouterAuth, baseUrl: String? = nil, token: String? = nil) -> AriaRouterAuth {
    var out = existing
    if let baseUrl { out.baseUrl = baseUrl }
    if let token { out.token = token }
    return out
}

private typealias AriaRouterInitFn = @convention(c) (UnsafePointer<CChar>?) -> OpaquePointer?
private typealias AriaRouterConnectFn = @convention(c) (UnsafePointer<CChar>?) -> OpaquePointer?
private typealias AriaRouterDestroyFn = @convention(c) (OpaquePointer?) -> Void
private typealias AriaRouterCompleteFn = @convention(c) (
    OpaquePointer?, UnsafePointer<CChar>?, UnsafePointer<CChar>?, UnsafeMutablePointer<CChar>?, Int
) -> Int32
private typealias AriaRouterBufOutFn = @convention(c) (
    OpaquePointer?, UnsafeMutablePointer<CChar>?, Int
) -> Int32
private typealias AriaRouterLastErrorFn = @convention(c) () -> UnsafePointer<CChar>?

/// Host-friendly Router over `libaria-router_ffi` via dlopen/dlsym (no XCFramework required).
public final class Router {
    private var dl: UnsafeMutableRawPointer?
    private var handle: OpaquePointer?
    private var auth = AriaRouterAuth()

    private var fnInit: AriaRouterInitFn!
    private var fnConnect: AriaRouterConnectFn!
    private var fnDestroy: AriaRouterDestroyFn!
    private var fnComplete: AriaRouterCompleteFn!
    private var fnModels: AriaRouterBufOutFn!
    private var fnLastRoute: AriaRouterBufOutFn!
    private var fnLastError: AriaRouterLastErrorFn!

    public init() {}

    deinit { close() }

    @discardableResult
    public func setup(baseUrl: String? = nil, token: String? = nil) -> Router {
        auth = applyRouterAuth(auth, baseUrl: baseUrl, token: token)
        return self
    }

    public func setupStatus() -> AriaRouterAuth { auth }

    @discardableResult
    public func setupClear() -> Router {
        auth = AriaRouterAuth()
        return self
    }

    private static func ariaHome() -> String {
        if let override = ProcessInfo.processInfo.environment["ARIA_COMPUTE_HOME"], !override.isEmpty {
            return override
        }
        return (NSHomeDirectory() as NSString).appendingPathComponent(".ariacompute")
    }

    private static func ffiLibNames() -> [String] {
        #if os(Windows)
        return ["aria-router_ffi.dll", "aria_router_ffi.dll"]
        #elseif os(macOS) || os(iOS)
        return ["libaria-router_ffi.dylib", "libaria_router_ffi.dylib"]
        #else
        return ["libaria-router_ffi.so", "libaria_router_ffi.so"]
        #endif
    }

    private static func resolveLibPath() throws -> String {
        if let env = ProcessInfo.processInfo.environment["ARIA_ROUTER_FFI_LIB"],
           !env.isEmpty,
           FileManager.default.fileExists(atPath: env) {
            return env
        }
        let libDir = (ariaHome() as NSString).appendingPathComponent("lib")
        for name in ffiLibNames() {
            let p = (libDir as NSString).appendingPathComponent(name)
            if FileManager.default.fileExists(atPath: p) { return p }
        }
        throw NSError(
            domain: "AriaRouter",
            code: 1,
            userInfo: [NSLocalizedDescriptionKey: "libaria-router_ffi not found; set ARIA_ROUTER_FFI_LIB"]
        )
    }

    private static func expandHome(_ path: String) -> String {
        if path == "~" { return NSHomeDirectory() }
        if path.hasPrefix("~/") {
            return (NSHomeDirectory() as NSString).appendingPathComponent(String(path.dropFirst(2)))
        }
        return path
    }

    private func ensureLoaded() throws {
        if dl != nil { return }
        let path = try Self.resolveLibPath()
        guard let h = dlopen(path, RTLD_NOW | RTLD_LOCAL) else {
            let msg = String(cString: dlerror())
            throw NSError(domain: "AriaRouter", code: 2, userInfo: [NSLocalizedDescriptionKey: msg])
        }
        dl = h
        func bind<T>(_ name: String) throws -> T {
            guard let sym = dlsym(h, name) else {
                throw NSError(
                    domain: "AriaRouter",
                    code: 3,
                    userInfo: [NSLocalizedDescriptionKey: "missing symbol \(name)"]
                )
            }
            return unsafeBitCast(sym, to: T.self)
        }
        fnInit = try bind("aria_router_init")
        fnConnect = try bind("aria_router_connect")
        fnDestroy = try bind("aria_router_destroy")
        fnComplete = try bind("aria_router_complete")
        fnModels = try bind("aria_router_models")
        fnLastRoute = try bind("aria_router_last_route")
        fnLastError = try bind("aria_router_last_error")
    }

    private func lastError(_ fallback: String) -> String {
        guard let p = fnLastError?() else { return fallback }
        let s = String(cString: p)
        return s.isEmpty ? fallback : s
    }

    /// Load YAML (`aria_router_init`). nil/empty → default `~/.ariacompute/router.yml`.
    @discardableResult
    public func load(_ configPath: String? = nil) throws -> Router {
        try ensureLoaded()
        close()
        var pathC: UnsafeMutablePointer<CChar>?
        defer { if let pathC { free(pathC) } }
        if let raw = configPath?.trimmingCharacters(in: .whitespacesAndNewlines), !raw.isEmpty {
            let expanded = Self.expandHome(raw)
            pathC = strdup(expanded)
        }
        guard let h = fnInit(pathC) else {
            throw NSError(
                domain: "AriaRouter",
                code: 4,
                userInfo: [NSLocalizedDescriptionKey: lastError("init failed")]
            )
        }
        handle = h
        return self
    }

    @discardableResult
    public func connect(_ baseUrl: String) throws -> Router {
        try ensureLoaded()
        close()
        let urlC = strdup(baseUrl)
        defer { free(urlC) }
        guard let h = fnConnect(urlC) else {
            throw NSError(
                domain: "AriaRouter",
                code: 5,
                userInfo: [NSLocalizedDescriptionKey: lastError("connect failed")]
            )
        }
        handle = h
        return self
    }

    public func close() {
        if let h = handle {
            fnDestroy?(h)
            handle = nil
        }
    }

    public func destroy() { close() }

    public func complete(messages: Any, options: Any = [String: Any]()) throws -> [String: Any] {
        guard let h = handle else {
            throw NSError(
                domain: "AriaRouter",
                code: 6,
                userInfo: [NSLocalizedDescriptionKey: "router not initialized"]
            )
        }
        let msgData = try JSONSerialization.data(withJSONObject: messages)
        let optData = try JSONSerialization.data(withJSONObject: options)
        let msg = String(data: msgData, encoding: .utf8) ?? "[]"
        let opt = String(data: optData, encoding: .utf8) ?? "{}"
        let msgC = strdup(msg)
        let optC = strdup(opt)
        defer {
            free(msgC)
            free(optC)
        }
        let cap = 256 * 1024
        let buf = UnsafeMutablePointer<CChar>.allocate(capacity: cap)
        defer { buf.deallocate() }
        buf.initialize(repeating: 0, count: cap)
        let rc = fnComplete(h, msgC, optC, buf, cap)
        if rc != 0 {
            throw NSError(
                domain: "AriaRouter",
                code: 7,
                userInfo: [NSLocalizedDescriptionKey: lastError("complete failed")]
            )
        }
        let text = String(cString: buf)
        let data = Data(text.utf8)
        let obj = try JSONSerialization.jsonObject(with: data)
        return obj as? [String: Any] ?? [:]
    }

    public func models() throws -> [String: Any] {
        guard let h = handle else {
            throw NSError(
                domain: "AriaRouter",
                code: 6,
                userInfo: [NSLocalizedDescriptionKey: "router not initialized"]
            )
        }
        let cap = 64 * 1024
        let buf = UnsafeMutablePointer<CChar>.allocate(capacity: cap)
        defer { buf.deallocate() }
        buf.initialize(repeating: 0, count: cap)
        let rc = fnModels(h, buf, cap)
        if rc != 0 {
            throw NSError(
                domain: "AriaRouter",
                code: 8,
                userInfo: [NSLocalizedDescriptionKey: lastError("models failed")]
            )
        }
        let text = String(cString: buf)
        let data = Data(text.utf8)
        let obj = try JSONSerialization.jsonObject(with: data)
        return obj as? [String: Any] ?? [:]
    }

    public func lastRoute() -> [String: Any] {
        guard let h = handle else { return [:] }
        let cap = 64 * 1024
        let buf = UnsafeMutablePointer<CChar>.allocate(capacity: cap)
        defer { buf.deallocate() }
        buf.initialize(repeating: 0, count: cap)
        let rc = fnLastRoute(h, buf, cap)
        if rc != 0 { return [:] }
        let text = String(cString: buf)
        guard let data = text.data(using: .utf8),
              let obj = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return [:]
        }
        return obj
    }
}

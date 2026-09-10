import Foundation
import XCTest
@testable import AriaRouter

final class AriaRouterTests: XCTestCase {
    func testSetupMemoryOnly() {
        var auth = AriaRouterAuth()
        auth = applyRouterAuth(auth, baseUrl: "http://127.0.0.1:8899", token: "t")
        XCTAssertEqual(auth.token, "t")
        let r = Router()
        r.setup(baseUrl: "http://127.0.0.1:8899", token: "t")
        XCTAssertEqual(r.setupStatus().token, "t")
        r.setupClear()
        XCTAssertEqual(r.setupStatus().token, "")
    }

    func testInitWhenFfiAvailable() throws {
        guard let lib = ProcessInfo.processInfo.environment["ARIA_ROUTER_FFI_LIB"],
              let cfg = ProcessInfo.processInfo.environment["ARIA_ROUTER_CONFIG"],
              !lib.isEmpty, !cfg.isEmpty,
              FileManager.default.fileExists(atPath: lib) else {
            throw XCTSkip("ARIA_ROUTER_FFI_LIB / ARIA_ROUTER_CONFIG unset")
        }
        let r = Router()
        try r.load(cfg)
        defer { r.close() }
        let models = try r.models()
        let raw = String(describing: models)
        XCTAssertTrue(raw.contains("semantic-auto"), raw)
        let out = try r.complete(
            messages: [["role": "user", "content": "hi"]],
            options: ["model": "ariacompute/semantic-auto"]
        )
        XCTAssertTrue(String(describing: out).contains("hello-from-router"))
        XCTAssertEqual(r.lastRoute()["layer"] as? String, "semantic")
    }
}

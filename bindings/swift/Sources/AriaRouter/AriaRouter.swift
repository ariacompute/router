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

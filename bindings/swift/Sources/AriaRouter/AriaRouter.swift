import Foundation

public struct AriarouterAuth {
    public var baseUrl: String = ""
    public var token: String = ""
    public init() {}
}

public func applyRouterAuth(_ existing: AriarouterAuth, baseUrl: String? = nil, token: String? = nil) -> AriarouterAuth {
    var out = existing
    if let baseUrl { out.baseUrl = baseUrl }
    if let token { out.token = token }
    return out
}

// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "AriaRouter",
    platforms: [
        .macOS(.v13),
        .iOS(.v15),
    ],
    products: [
        .library(name: "AriaRouter", targets: ["AriaRouter"]),
    ],
    targets: [
        .target(
            name: "AriaRouter",
            path: "Sources/AriaRouter"
        ),
        .testTarget(
            name: "AriaRouterTests",
            dependencies: ["AriaRouter"],
            path: "Tests/AriaRouterTests"
        ),
    ]
)

// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "AriaComputeRouter",
    platforms: [
        .macOS(.v13),
        .iOS(.v15),
    ],
    products: [
        .library(name: "AriaComputeRouter", targets: ["AriaComputeRouter"]),
    ],
    targets: [
        .target(
            name: "AriaComputeRouter",
            path: "Sources/AriaComputeRouter"
        ),
        .testTarget(
            name: "AriaComputeRouterTests",
            dependencies: ["AriaComputeRouter"],
            path: "Tests/AriaComputeRouterTests"
        ),
    ]
)

// swift-tools-version:5.9
import PackageDescription

let package = Package(
    name: "DaanioKit",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        .library(name: "DaanioKit", targets: ["DaanioKit"])
    ],
    targets: [
        .target(
            name: "DaanioKit",
            swiftSettings: [.enableUpcomingFeature("StrictConcurrency")]
        ),
        .testTarget(
            name: "DaanioKitTests",
            dependencies: ["DaanioKit"]
        ),
    ]
)

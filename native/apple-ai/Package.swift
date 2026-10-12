// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "InvisibleAppleAI",
    platforms: [.macOS(.v26)],
    products: [.executable(name: "invisible-apple-ai", targets: ["InvisibleAppleAI"])],
    targets: [.executableTarget(name: "InvisibleAppleAI")]
)

// swift-tools-version: 6.0
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
// UltraVoxMacOSBridge
// Native macOS capabilities for the Rust/Tauri desktop app.
//
// The package exposes a C-compatible ABI so the Rust bridge crate can call
// Accessibility, CoreAudio, AppKit, ScreenCaptureKit, and FluidAudio/Core ML.
//
//  License: MIT (see /LICENSE in the repository root)

import PackageDescription

let package = Package(
    name: "UltraVoxMacOSBridge",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "UltraVoxMacOSBridge",
            type: .static,
            targets: ["UltraVoxMacOSBridge"]
        )
    ],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.4")
    ],
    targets: [
        .target(
            name: "UltraVoxMacOSBridge",
            dependencies: [
                .product(name: "FluidAudio", package: "FluidAudio")
            ],
            path: "Sources/UltraVoxMacOSBridge",
            exclude: ["include"]
        ),
        .testTarget(
            name: "UltraVoxMacOSBridgeTests",
            dependencies: ["UltraVoxMacOSBridge"],
            path: "Tests/UltraVoxMacOSBridgeTests"
        )
    ]
)

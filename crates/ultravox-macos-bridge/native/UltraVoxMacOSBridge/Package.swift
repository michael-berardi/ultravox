// swift-tools-version: 6.0
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
//  UltraVoxMacOSBridge
//  Minimal macOS-native bridge for Rust/Tauri UltraVox parity.
//
//  This package is intentionally isolated from the existing UltraVox Xcode
//  project. It exposes a C-compatible ABI so that the Rust crate in
//  crates/ultravox-macos-bridge can call into macOS-specific frameworks
//  (AX, CoreGraphics, AppKit, FluidAudio / CoreML) without altering the Swift baseline app.
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

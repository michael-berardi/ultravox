// swift-tools-version: 6.0
// The swift-tools-version declares the minimum version of Swift required to build this package.
//
//  DictatorMacOSBridge
//  Minimal macOS-native bridge for Rust/Tauri Dictator parity.
//
//  This package is intentionally isolated from the existing Dictator Xcode
//  project. It exposes a C-compatible ABI so that the Rust crate in
//  crates/dictator-macos-bridge can call into macOS-specific frameworks
//  (AX, CoreGraphics, AppKit, FluidAudio / CoreML) without altering the Swift baseline app.
//
//  License: MIT (see /LICENSE in the repository root)

import PackageDescription

let package = Package(
    name: "DictatorMacOSBridge",
    platforms: [
        .macOS(.v14)
    ],
    products: [
        .library(
            name: "DictatorMacOSBridge",
            type: .static,
            targets: ["DictatorMacOSBridge"]
        )
    ],
    dependencies: [
        .package(url: "https://github.com/FluidInference/FluidAudio.git", exact: "0.15.4")
    ],
    targets: [
        .target(
            name: "DictatorMacOSBridge",
            dependencies: [
                .product(name: "FluidAudio", package: "FluidAudio")
            ],
            path: "Sources/DictatorMacOSBridge",
            exclude: ["include"]
        ),
        .testTarget(
            name: "DictatorMacOSBridgeTests",
            dependencies: ["DictatorMacOSBridge"],
            path: "Tests/DictatorMacOSBridgeTests"
        )
    ]
)

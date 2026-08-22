import XCTest
@testable import UltraVoxMacOSBridge

final class UltraVoxMacOSBridgeTests: XCTestCase {

    // MARK: - Modifier discrimination

    func testPhysicalModifierAcceptsSupportedModifiers() {
        let cases: [(String, UInt16)] = [
            ("leftOption", 58),
            ("rightOption", 61),
            ("rightCommand", 54),
        ]

        for (raw, expectedKeyCode) in cases {
            let modifier = PhysicalModifier(rawValue: raw)
            XCTAssertNotNil(modifier, "\(raw) should be accepted")
            XCTAssertEqual(modifier?.keyCode, expectedKeyCode)
            XCTAssertEqual(modifier?.rawValue, raw)
        }
    }

    func testPhysicalModifierRejectsShiftVariants() {
        let shiftNames = ["leftShift", "rightShift", "shift", "SHIFT", "LeftShift"]
        for raw in shiftNames {
            XCTAssertNil(
                PhysicalModifier(rawValue: raw),
                "\(raw) must not be usable as a modifier-only hotkey"
            )
        }
    }

    func testPhysicalModifierRejectsUnsupportedModifiers() {
        let unsupported = [
            "none", "", "leftCommand", "command", "control", "option", "alt",
        ]
        for raw in unsupported {
            XCTAssertNil(
                PhysicalModifier(rawValue: raw),
                "\(raw) should be rejected"
            )
        }
    }

    // MARK: - Coordinate conversion

    func testConvertAXPointToCocoaFlipsYAgainstPrimaryScreen() {
        let primaryFrame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let axPoint = CGPoint(x: 100, y: 50)
        let cocoa = FocusUtils.convertAXPointToCocoa(axPoint, primaryScreenFrame: primaryFrame)

        XCTAssertEqual(cocoa.x, 100)
        XCTAssertEqual(cocoa.y, 850) // 900 - 50
    }

    func testConvertAXPointToCocoaHandlesDisplaysAbovePrimary() {
        // Primary is 900pt tall; a 1080pt display above it yields negative AX y values.
        // A point 100pt below the top of that display has AX y = -(1080 - 100) = -980.
        let primaryFrame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let axPoint = CGPoint(x: 0, y: -980)
        let cocoa = FocusUtils.convertAXPointToCocoa(axPoint, primaryScreenFrame: primaryFrame)

        XCTAssertEqual(cocoa.x, 0)
        XCTAssertEqual(cocoa.y, 1880) // 900 - (-980) == 900 + 1080 - 100
    }

    func testConvertAXPointToCocoaHandlesDisplaysToTheRight() {
        let primaryFrame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        // A point 20pt below the top of a screen to the right of the primary.
        let axPoint = CGPoint(x: 1500, y: 20)
        let cocoa = FocusUtils.convertAXPointToCocoa(axPoint, primaryScreenFrame: primaryFrame)

        XCTAssertEqual(cocoa.x, 1500)
        XCTAssertEqual(cocoa.y, 880) // 900 - 20
    }

    // MARK: - Indicator clipping avoidance

    func testClampedOriginCentersAbovePointWhenThereIsRoom() {
        let visibleFrame = NSRect(x: 0, y: 0, width: 1440, height: 900)
        let point = NSPoint(x: 500, y: 200)
        let size = NSSize(width: 128, height: 28)

        let origin = IndicatorGeometry.clampedOrigin(
            point: point,
            size: size,
            visibleFrame: visibleFrame,
            verticalOffset: 14,
            padding: 8
        )

        XCTAssertEqual(origin.x, 500 - 64)
        XCTAssertEqual(origin.y, 200 + 14)
    }

    func testClampedOriginFlipsBelowWhenAboveWouldClip() {
        let visibleFrame = NSRect(x: 0, y: 0, width: 1440, height: 100)
        let point = NSPoint(x: 500, y: 90)
        let size = NSSize(width: 128, height: 28)

        let origin = IndicatorGeometry.clampedOrigin(
            point: point,
            size: size,
            visibleFrame: visibleFrame,
            verticalOffset: 14,
            padding: 8
        )

        // desired top = 90 + 14 + 28 + 8 = 140 > 100, so it should flip below the caret
        XCTAssertEqual(origin.y, 90 - 28 - 14)
        XCTAssertEqual(origin.x, 500 - 64)
    }

    func testClampedOriginClampsToVisibleFrame() {
        let visibleFrame = NSRect(x: 100, y: 50, width: 500, height: 400)
        let point = NSPoint(x: 0, y: 0)
        let size = NSSize(width: 128, height: 28)

        let origin = IndicatorGeometry.clampedOrigin(
            point: point,
            size: size,
            visibleFrame: visibleFrame,
            verticalOffset: 14,
            padding: 8
        )

        XCTAssertEqual(origin.x, visibleFrame.minX + 8)
        XCTAssertEqual(origin.y, visibleFrame.minY + 8)
    }

    // MARK: - Media panel support

    func testMediaTransportCommandsMapToStableSendCommandCodes() {
        XCTAssertEqual(MediaTransportCommand(rawValue: "play_pause")?.sendCommandCode, 2)
        XCTAssertEqual(MediaTransportCommand(rawValue: "next")?.sendCommandCode, 4)
        XCTAssertEqual(MediaTransportCommand(rawValue: "previous")?.sendCommandCode, 5)
        XCTAssertNil(MediaTransportCommand(rawValue: "stop"))
        XCTAssertNil(MediaTransportCommand(rawValue: "PLAY_PAUSE"))
    }

    func testPlayingStateDerivesFromPlaybackRate() {
        XCTAssertTrue(
            NowPlayingSnapshot.playingState(
                from: ["kMRMediaRemoteNowPlayingInfoPlaybackRate": 1.5])!)
        XCTAssertFalse(
            NowPlayingSnapshot.playingState(
                from: ["kMRMediaRemoteNowPlayingInfoPlaybackRate": 0.0])!)
        XCTAssertNil(NowPlayingSnapshot.playingState(from: [:]))
    }

    func testNowPlayingTimeParsingRejectsMalformedValues() {
        XCTAssertEqual(
            NowPlayingSnapshot.time(
                "kMRMediaRemoteNowPlayingInfoElapsedTime",
                from: ["kMRMediaRemoteNowPlayingInfoElapsedTime": 12.5]),
            12.5)
        XCTAssertNil(
            NowPlayingSnapshot.time(
                "kMRMediaRemoteNowPlayingInfoDuration",
                from: ["kMRMediaRemoteNowPlayingInfoDuration": -1.0]))
        XCTAssertNil(
            NowPlayingSnapshot.time(
                "kMRMediaRemoteNowPlayingInfoDuration",
                from: ["kMRMediaRemoteNowPlayingInfoDuration": "unknown"]))
    }

    func testBundleFamilyMatchingIsConservative() {
        XCTAssertTrue(BundleFamily.matches(
            "com.google.Chrome", "com.google.Chrome.helper.renderer"))
        XCTAssertTrue(BundleFamily.matches(
            "com.apple.Safari", "com.apple.WebKit.WebContent"))
        XCTAssertTrue(BundleFamily.matches("com.example.player", "com.example.player"))
        XCTAssertFalse(BundleFamily.matches(
            "com.google.Chrome", "com.google.Chromeish.helper"))
        XCTAssertFalse(BundleFamily.matches("com.google.Chrome", "com.apple.Safari"))
        XCTAssertFalse(BundleFamily.matches(nil, "com.google.Chrome"))
    }

    func testTransportCapabilityResolutionUsesSupportedAndEnabledCommands() {
        let resolved = MediaTransportCapabilities.resolve(
            sendAvailable: true,
            supported: Set([4, 5]),
            enabled: Set([4]))
        XCTAssertTrue(resolved.playPause)
        XCTAssertFalse(resolved.previous)
        XCTAssertTrue(resolved.next)

        let unavailable = MediaTransportCapabilities.resolve(
            sendAvailable: true,
            supported: nil,
            enabled: nil)
        XCTAssertTrue(unavailable.playPause)
        XCTAssertFalse(unavailable.previous)
        XCTAssertFalse(unavailable.next)

        let unsendable = MediaTransportCapabilities.resolve(
            sendAvailable: false,
            supported: Set([4, 5]),
            enabled: Set([4, 5]))
        XCTAssertFalse(unsendable.playPause)
        XCTAssertFalse(unsendable.previous)
        XCTAssertFalse(unsendable.next)
    }

    func testActiveAudioProcessBridgeReturnsKnownValue() {
        // Capture-free probe must return a well-formed signal in any
        // environment: 1 when another process runs output audio, else 0.
        var processID: pid_t = 0
        var appName: UnsafeMutablePointer<CChar>? = nil
        var bundleId: UnsafeMutablePointer<CChar>? = nil
        let result = ultravox_macos_bridge_active_audio_process(
            &processID, &appName, &bundleId)
        XCTAssertTrue(result == 0 || result == 1)
        if result == 1 { XCTAssertGreaterThan(processID, 0) }
        if let appName { ultravox_macos_bridge_free_string(appName) }
        if let bundleId { ultravox_macos_bridge_free_string(bundleId) }
    }
    func testMicrophoneAuthorizationStatusReturnsKnownValue() {
        let status = ultravox_macos_bridge_microphone_authorization_status()
        XCTAssertTrue((0...3).contains(status))
    }

}

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
    func testMicrophoneAuthorizationStatusReturnsKnownValue() {
        let status = ultravox_macos_bridge_microphone_authorization_status()
        XCTAssertTrue((0...3).contains(status))
    }

}

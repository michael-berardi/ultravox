import XCTest
@testable import UltraVoxMacOSBridge

final class UltraVoxMacOSBridgeTests: XCTestCase {
    func testBridgeVersionIsPublic() {
        let version = ultravox_macos_bridge_version()
        XCTAssertNotNil(version)
        ultravox_macos_bridge_free_string(version)
    }

    func testKeyCombinationAcceptsBareSpaceForHoldToRecord() {
        let shortcut = KeyCombination.parse("Space")
        XCTAssertEqual(shortcut?.raw, "Space")
        XCTAssertNil(shortcut?.modifier)
        XCTAssertEqual(shortcut?.keyCode, 49)
        XCTAssertNil(KeyCombination.parse("A"))
    }
}

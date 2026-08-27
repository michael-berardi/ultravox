import XCTest
@testable import UltraVoxMacOSBridge

final class UltraVoxMacOSBridgeTests: XCTestCase {
    func testBridgeVersionIsPublic() {
        let version = ultravox_macos_bridge_version()
        XCTAssertNotNil(version)
        ultravox_macos_bridge_free_string(version)
    }
}

import CoreImage
import CoreImage.CIFilterBuiltins
import XCTest

final class PairingScanTests: XCTestCase {
    private func qr(_ text: String) -> CGImage {
        let filter = CIFilter.qrCodeGenerator()
        filter.message = Data(text.utf8)
        let image = filter.outputImage!.transformed(by: CGAffineTransform(scaleX: 8, y: 8))
        return CIContext().createCGImage(image, from: image.extent)!
    }

    func testFindsThePairingCode() {
        let code = "YTTT-PAIR:1:ABCDEF234567"
        XCTAssertEqual(pairingCode(in: qr(code)), code)
    }

    func testIgnoresOtherCodesAndPlainImages() {
        XCTAssertNil(pairingCode(in: qr("https://example.com")))
        let blank = CIImage(color: .white).cropped(to: CGRect(x: 0, y: 0, width: 200, height: 200))
        XCTAssertNil(pairingCode(in: CIContext().createCGImage(blank, from: blank.extent)!))
    }
}

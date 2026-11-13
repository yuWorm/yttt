import CoreImage
import SwiftUI
import UIKit
import VisionKit

private let pairingPrefix = "YTTT-PAIR:"

/// The first pairing code (`YTTT-PAIR:…`) in a QR code in `image`.
func pairingCode(in image: CGImage) -> String? {
    let detector = CIDetector(ofType: CIDetectorTypeQRCode, context: nil,
                              options: [CIDetectorAccuracy: CIDetectorAccuracyHigh])
    return detector?.features(in: CIImage(cgImage: image))
        .compactMap { ($0 as? CIQRCodeFeature)?.messageString }
        .first { $0.hasPrefix(pairingPrefix) }
}

/// The camera, looking for a pairing QR code; hands over the first it reads.
struct PairingScanner: UIViewControllerRepresentable {
    let onCode: (String) -> Void

    static var available: Bool { DataScannerViewController.isSupported && DataScannerViewController.isAvailable }

    func makeUIViewController(context: Context) -> DataScannerViewController {
        let scanner = DataScannerViewController(recognizedDataTypes: [.barcode(symbologies: [.qr])],
                                                qualityLevel: .balanced, isHighlightingEnabled: true)
        scanner.delegate = context.coordinator
        try? scanner.startScanning()
        return scanner
    }

    func updateUIViewController(_ scanner: DataScannerViewController, context: Context) {}

    func makeCoordinator() -> Coordinator { Coordinator(onCode: onCode) }

    final class Coordinator: NSObject, DataScannerViewControllerDelegate {
        let onCode: (String) -> Void
        private var done = false
        init(onCode: @escaping (String) -> Void) { self.onCode = onCode }

        func dataScanner(_ scanner: DataScannerViewController, didAdd items: [RecognizedItem],
                         allItems: [RecognizedItem]) {
            guard !done else { return }
            for case .barcode(let barcode) in items {
                if let text = barcode.payloadStringValue, text.hasPrefix(pairingPrefix) {
                    done = true
                    scanner.stopScanning()
                    onCode(text)
                    return
                }
            }
        }
    }
}

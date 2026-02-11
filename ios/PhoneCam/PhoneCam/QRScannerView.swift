import AVFoundation
import SwiftUI
import UIKit

struct QRScannerView: View {
    let onScannedUri: (String) -> Void

    @Environment(\.dismiss) private var dismiss
    @StateObject private var scannerController = QRScannerController()

    var body: some View {
        ZStack {
            QRScannerPreview(session: scannerController.session)
                .ignoresSafeArea()

            VStack(spacing: 16) {
                Text(scannerController.statusText)
                    .font(.subheadline.weight(.semibold))
                    .foregroundColor(.white)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(Color.black.opacity(0.7), in: Capsule())

                if scannerController.permissionDenied {
                    Text("Enable camera permission in Settings to scan PhoneCam QR codes.")
                        .font(.footnote)
                        .foregroundColor(.white)
                        .multilineTextAlignment(.center)
                        .padding(14)
                        .background(Color.black.opacity(0.7), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                        .padding(.horizontal)
                }

                Spacer()

                Button {
                    dismiss()
                } label: {
                    Text("Cancel")
                        .font(.headline)
                        .foregroundColor(.white)
                        .padding(.horizontal, 26)
                        .padding(.vertical, 12)
                        .background(Color.black.opacity(0.7), in: Capsule())
                }
                .padding(.bottom, 30)
            }
        }
        .background(Color.black.ignoresSafeArea())
        .onAppear {
            scannerController.onDetectedUri = { scannedUri in
                onScannedUri(scannedUri)
                dismiss()
            }
            scannerController.start()
        }
        .onDisappear {
            scannerController.stop()
        }
    }
}

private struct QRScannerPreview: UIViewRepresentable {
    let session: AVCaptureSession

    func makeUIView(context: Context) -> QRScannerPreviewContainerView {
        let view = QRScannerPreviewContainerView()
        view.previewLayer.videoGravity = .resizeAspectFill
        view.previewLayer.session = session
        return view
    }

    func updateUIView(_ uiView: QRScannerPreviewContainerView, context: Context) {
        uiView.previewLayer.session = session
    }
}

private final class QRScannerPreviewContainerView: UIView {
    override class var layerClass: AnyClass {
        AVCaptureVideoPreviewLayer.self
    }

    var previewLayer: AVCaptureVideoPreviewLayer {
        layer as! AVCaptureVideoPreviewLayer
    }
}

private final class QRScannerController: NSObject, ObservableObject, AVCaptureMetadataOutputObjectsDelegate {
    let session = AVCaptureSession()

    @Published var statusText = "Point camera at a PhoneCam QR code"
    @Published var permissionDenied = false

    var onDetectedUri: ((String) -> Void)?

    private let sessionQueue = DispatchQueue(label: "com.phonecam.ios.qr.session")
    private var isConfigured = false
    private var didEmitResult = false

    func start() {
        let authorizationStatus = AVCaptureDevice.authorizationStatus(for: .video)
        switch authorizationStatus {
        case .authorized:
            configureSessionIfNeededAndStart()
        case .notDetermined:
            AVCaptureDevice.requestAccess(for: .video) { [weak self] granted in
                guard let self else {
                    return
                }

                DispatchQueue.main.async {
                    self.permissionDenied = !granted
                }

                if granted {
                    self.configureSessionIfNeededAndStart()
                }
            }
        case .denied, .restricted:
            DispatchQueue.main.async {
                self.permissionDenied = true
                self.statusText = "Camera permission required"
            }
        @unknown default:
            DispatchQueue.main.async {
                self.permissionDenied = true
                self.statusText = "Camera permission required"
            }
        }
    }

    func stop() {
        sessionQueue.async {
            if self.session.isRunning {
                self.session.stopRunning()
            }
            self.didEmitResult = false
        }
    }

    private func configureSessionIfNeededAndStart() {
        sessionQueue.async {
            if !self.isConfigured {
                self.session.beginConfiguration()
                self.session.sessionPreset = .high

                guard
                    let camera = AVCaptureDevice.default(.builtInWideAngleCamera, for: .video, position: .back)
                        ?? AVCaptureDevice.default(for: .video),
                    let input = try? AVCaptureDeviceInput(device: camera),
                    self.session.canAddInput(input)
                else {
                    self.session.commitConfiguration()
                    DispatchQueue.main.async {
                        self.statusText = "Unable to access camera"
                    }
                    return
                }

                self.session.addInput(input)

                let metadataOutput = AVCaptureMetadataOutput()
                guard self.session.canAddOutput(metadataOutput) else {
                    self.session.commitConfiguration()
                    DispatchQueue.main.async {
                        self.statusText = "Unable to configure scanner"
                    }
                    return
                }

                self.session.addOutput(metadataOutput)
                metadataOutput.setMetadataObjectsDelegate(self, queue: .main)
                metadataOutput.metadataObjectTypes = [.qr]

                self.session.commitConfiguration()
                self.isConfigured = true
            }

            if !self.session.isRunning {
                self.session.startRunning()
            }

            DispatchQueue.main.async {
                self.statusText = "Point camera at a PhoneCam QR code"
            }
        }
    }

    func metadataOutput(
        _ output: AVCaptureMetadataOutput,
        didOutput metadataObjects: [AVMetadataObject],
        from connection: AVCaptureConnection
    ) {
        guard !didEmitResult else {
            return
        }

        let scannedUri =
            metadataObjects
                .compactMap { $0 as? AVMetadataMachineReadableCodeObject }
                .first { $0.type == .qr }?
                .stringValue

        guard let scannedUri, scannedUri.hasPrefix("phonecam://") else {
            return
        }

        didEmitResult = true
        statusText = "QR code detected"
        onDetectedUri?(scannedUri)
    }
}

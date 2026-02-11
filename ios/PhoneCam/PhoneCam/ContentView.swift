import AVFoundation
import SwiftUI
import UIKit

struct ContentView: View {
    @ObservedObject var cameraController: CameraController
    @ObservedObject var streamManager: StreamManager
    @State private var isShowingQrScanner = false

    var body: some View {
        ZStack {
            CameraPreviewView(session: cameraController.session)
                .ignoresSafeArea()
                .overlay(permissionOverlay)

            VStack {
                Spacer()

                VStack(alignment: .leading, spacing: 12) {
                    HStack(spacing: 8) {
                        Circle()
                            .fill(streamManager.isConnected ? Color.green : Color.orange)
                            .frame(width: 10, height: 10)

                        Text(streamManager.statusText)
                            .foregroundColor(.white)
                            .font(.subheadline)

                        Spacer()

                        Text(cameraController.isSessionRunning ? "Camera On" : "Camera Off")
                            .foregroundColor(.white.opacity(0.85))
                            .font(.footnote)
                    }

                    if let interruption = cameraController.interruptionDescription {
                        Text(interruption)
                            .font(.footnote)
                            .foregroundColor(.yellow)
                    }

                    Picker(
                        "Resolution",
                        selection: Binding(
                            get: { streamManager.selectedResolution },
                            set: { streamManager.setResolution($0) }
                        )
                    ) {
                        ForEach(CaptureResolution.allCases) { resolution in
                            Text(resolution.rawValue).tag(resolution)
                        }
                    }
                    .pickerStyle(.segmented)

                    Button {
                        isShowingQrScanner = true
                    } label: {
                        HStack(spacing: 8) {
                            Image(systemName: "qrcode.viewfinder")
                            Text("Scan QR Code")
                        }
                        .font(.subheadline.weight(.semibold))
                        .foregroundColor(.white)
                        .padding(.vertical, 10)
                        .frame(maxWidth: .infinity)
                        .background(Color.white.opacity(0.15), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                }
                .padding(16)
                .background(.ultraThinMaterial, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                .padding()
            }
        }
        .background(Color.black.ignoresSafeArea())
        .onAppear {
            streamManager.startStreaming()
        }
        .onDisappear {
            streamManager.stopStreaming()
        }
        .sheet(isPresented: $isShowingQrScanner) {
            QRScannerView(onScannedUri: { scannedUri in
                _ = streamManager.connectUsingQrUri(scannedUri)
            })
        }
    }

    @ViewBuilder
    private var permissionOverlay: some View {
        if cameraController.authorizationStatus == .denied || cameraController.authorizationStatus == .restricted {
            VStack(spacing: 10) {
                Image(systemName: "camera.fill")
                    .font(.title)
                    .foregroundColor(.white)

                Text("Camera Access Required")
                    .font(.headline)
                    .foregroundColor(.white)

                Text("Enable camera permission in Settings to preview and stream video.")
                    .multilineTextAlignment(.center)
                    .font(.subheadline)
                    .foregroundColor(.white.opacity(0.9))
            }
            .padding(20)
            .background(Color.black.opacity(0.7), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
            .padding()
        }
    }
}

struct ContentView_Previews: PreviewProvider {
    static var previews: some View {
        let cameraController = CameraController()
        let streamManager = StreamManager(cameraController: cameraController)
        ContentView(cameraController: cameraController, streamManager: streamManager)
    }
}

private struct CameraPreviewView: UIViewRepresentable {
    let session: AVCaptureSession

    func makeUIView(context: Context) -> PreviewContainerView {
        let view = PreviewContainerView()
        view.previewLayer.videoGravity = .resizeAspectFill
        view.previewLayer.session = session
        return view
    }

    func updateUIView(_ uiView: PreviewContainerView, context: Context) {
        uiView.previewLayer.session = session
    }
}

private final class PreviewContainerView: UIView {
    override class var layerClass: AnyClass {
        AVCaptureVideoPreviewLayer.self
    }

    var previewLayer: AVCaptureVideoPreviewLayer {
        layer as! AVCaptureVideoPreviewLayer
    }
}

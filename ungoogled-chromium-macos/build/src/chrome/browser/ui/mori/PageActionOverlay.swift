import SwiftUI
import CoreImage.CIFilterBuiltins

struct QRCodeSheet: Identifiable, Equatable {
    let id = UUID()
    var url: String
    var title: String
}

struct PageActionOverlay: View {
    @ObservedObject var store: BrowserStore
    @Environment(\.palette) private var p

    var body: some View {
        ZStack {
            if let sheet = store.qrCodeSheet {
                Color.black.opacity(0.18)
                    .ignoresSafeArea()
                    .onTapGesture { store.closeQRCode() }

                QRCodeCard(sheet: sheet) {
                    store.closeQRCode()
                }
                .transition(.scale(scale: 0.96).combined(with: .opacity))
            }
        }
        .animation(Motion.reveal, value: store.qrCodeSheet)
    }
}

private struct QRCodeCard: View {
    let sheet: QRCodeSheet
    let close: () -> Void
    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        VStack(spacing: 14) {
            HStack(spacing: 10) {
                Icon(name: "qrcode", size: 18, weight: .semibold)
                    .foregroundStyle(p.primary.color)
                VStack(alignment: .leading, spacing: 2) {
                    Text(sheet.title.isEmpty ? "QR Code" : sheet.title)
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                        .lineLimit(1)
                    Text(sheet.url)
                        .font(Typography.ui(Typography.small))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(1)
                        .truncationMode(.middle)
                        .help(sheet.url)
                }
                Spacer(minLength: 0)
                Button(action: close) {
                    Icon(name: "xmark", size: 11, weight: .bold)
                        .foregroundStyle(p.mutedForeground.color)
                        .frame(width: 24, height: 24)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Close")
            }

            if let image = QRCodeRenderer.image(for: sheet.url) {
                Image(nsImage: image)
                    .interpolation(.none)
                    .resizable()
                    .frame(width: 220, height: 220)
                    .padding(14)
                    .background(Color.white, in: RoundedRectangle(cornerRadius: Radius.md, style: .continuous))
            } else {
                Text("Could not generate QR code")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.mutedForeground.color)
                    .frame(width: 220, height: 220)
            }

            HStack(spacing: 8) {
                Button {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(sheet.url, forType: .string)
                    ToastCenter.shared.show("Link copied", icon: "link", style: .success)
                } label: {
                    Label("Copy Link", systemImage: "link")
                        .font(Typography.ui(Typography.base, weight: .medium))
                }
                .buttonStyle(.plain)

                Spacer()

                Button("Done", action: close)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .buttonStyle(.plain)
                    .keyboardShortcut(.defaultAction)
            }
            .foregroundStyle(p.foreground.color)
        }
        .padding(16)
        .frame(width: 330)
        .background(p.popover.color, in: RoundedRectangle(cornerRadius: Radius.lg, style: .continuous))
        .overlay(
            RoundedRectangle(cornerRadius: Radius.lg, style: .continuous)
                .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
        )
        .elevation(.popover, scheme)
    }
}

private enum QRCodeRenderer {
    private static let context = CIContext()
    private static let filter = CIFilter.qrCodeGenerator()

    static func image(for text: String) -> NSImage? {
        filter.message = Data(text.utf8)
        filter.correctionLevel = "M"
        guard let output = filter.outputImage else { return nil }
        let scaled = output.transformed(by: CGAffineTransform(scaleX: 12, y: 12))
        guard let cgImage = context.createCGImage(scaled, from: scaled.extent) else {
            return nil
        }
        return NSImage(cgImage: cgImage, size: NSSize(width: 220, height: 220))
    }
}

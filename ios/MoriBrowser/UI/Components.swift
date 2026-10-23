import SwiftUI
import UIKit

struct PineMark: View {
    var body: some View {
        Image("MoriMark")
            .resizable()
            .renderingMode(.template)
            .scaledToFit()
            .accessibilityHidden(true)
    }
}

struct SymbolIcon: View {
    let name: String
    var size: CGFloat = 16

    var body: some View {
        if name.unicodeScalars.contains(where: { !$0.isASCII }) {
            Text(name).font(.system(size: size))
        } else {
            Image(systemName: name)
                .font(.system(size: size, weight: .medium))
        }
    }
}

struct FaviconView: View {
    let urlString: String?
    var title: String = "New Tab"
    var isPrivate = false
    var size: CGFloat = 16
    @Environment(\.moriPalette) private var palette

    private var host: String? { WebAddress.host(of: urlString) }

    var body: some View {
        Group {
            if let host, !isPrivate, let image = FaviconLoader.shared.image(for: host) {
                Image(uiImage: image)
                    .resizable()
                    .scaledToFit()
            } else {
                Text(WebAddress.monogramLetter(urlString: urlString, title: title))
                    .font(.system(size: size * 0.55, weight: .semibold))
                    .foregroundStyle(palette.foreground)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                    .background(palette.tile)
            }
        }
        .frame(width: size, height: size)
        .clipShape(RoundedRectangle(cornerRadius: size * 0.22, style: .continuous))
        .task(id: host) {
            guard let host, !isPrivate else { return }
            await FaviconLoader.shared.load(host: host)
        }
    }
}

struct ToastPill: View {
    let text: String
    @Environment(\.moriPalette) private var palette

    var body: some View {
        Text(text)
            .font(.subheadline.weight(.semibold))
            .foregroundStyle(palette.background)
            .padding(.horizontal, 14)
            .padding(.vertical, 8)
            .background(palette.foreground, in: Capsule())
            .accessibilityLabel(text)
    }
}

struct ActivityView: UIViewControllerRepresentable {
    let items: [Any]

    func makeUIViewController(context: Context) -> UIActivityViewController {
        UIActivityViewController(activityItems: items, applicationActivities: nil)
    }

    func updateUIViewController(_ controller: UIActivityViewController, context: Context) {}
}

struct IconButton: View {
    let systemName: String
    var label: String
    var enabled = true
    let action: () -> Void
    @Environment(\.moriPalette) private var palette

    var body: some View {
        Button(action: action) {
            Image(systemName: systemName)
                .font(.body.weight(.medium))
                .frame(width: 44, height: 44)
                .foregroundStyle(palette.foreground)
                .opacity(enabled ? 1 : 0.35)
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(!enabled)
        .accessibilityLabel(label)
    }
}

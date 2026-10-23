import SwiftUI

struct OnboardingView: View {
    var start: () -> Void
    @Environment(\.moriPalette) private var palette

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            Spacer(minLength: 24)
            PineMark()
                .frame(width: 48, height: 32, alignment: .leading)
                .foregroundStyle(palette.foreground)
            Text("A calm place\nto browse.")
                .font(.system(size: 40, weight: .semibold))
                .foregroundStyle(palette.foreground)
                .padding(.top, 22)
            VStack(alignment: .leading, spacing: 16) {
                line("square.on.square", "Open tabs stay in one list.")
                line("star", "Favorites stay put, above today's tabs.")
                line("magnifyingglass", "Search, a link, or a command. One bar.")
            }
            .padding(.top, 28)
            Spacer(minLength: 32)
            Button(action: start) {
                Text("Start browsing")
                    .font(.system(size: 17, weight: .semibold))
                    .frame(maxWidth: .infinity)
                    .frame(height: 52)
                    .foregroundStyle(palette.background)
                    .background(palette.foreground, in: Capsule())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("startBrowsing")
        }
        .padding(.horizontal, 28)
        .padding(.bottom, 20)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(palette.background.ignoresSafeArea())
    }

    private func line(_ symbol: String, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Image(systemName: symbol)
                .font(.system(size: 15, weight: .medium))
                .frame(width: 22)
                .foregroundStyle(palette.mutedForeground)
            Text(text)
                .font(.system(size: 16))
                .foregroundStyle(palette.foreground)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

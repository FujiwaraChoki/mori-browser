import SwiftUI

struct ReaderView: View {
    let article: ReaderArticle
    @Environment(\.dismiss) private var dismiss
    @Environment(\.colorScheme) private var colorScheme
    @State private var size: CGFloat = 20

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    Text(article.title)
                        .font(.system(size: size + 8, weight: .semibold, design: .serif))
                    if !article.byline.isEmpty {
                        Text(article.byline)
                            .font(.subheadline)
                            .foregroundStyle(.secondary)
                    }
                    ForEach(article.blocks) { block in
                        Text(block.text)
                            .font(font(for: block.tag))
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
                .padding(24)
            }
            .background(paper)
            .navigationTitle("Reader")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Done") { dismiss() }
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { size = min(size + 2, 28) } label: {
                        Image(systemName: "textformat.size.larger")
                    }
                    .accessibilityLabel("Larger text")
                }
                ToolbarItem(placement: .topBarTrailing) {
                    Button { size = max(size - 2, 16) } label: {
                        Image(systemName: "textformat.size.smaller")
                    }
                    .accessibilityLabel("Smaller text")
                }
            }
        }
        .tint(ink)
    }

    private var paper: Color {
        colorScheme == .dark ? Color(hex: 0x1C1B19) : Color(hex: 0xF6F1E7)
    }

    private var ink: Color {
        colorScheme == .dark ? Color(hex: 0xF3EEE4) : Color(hex: 0x221E19)
    }

    private func font(for tag: String) -> Font {
        switch tag {
        case "h1":
            return .system(size: size + 6, weight: .semibold, design: .serif)
        case "h2", "h3":
            return .system(size: size + 2, weight: .semibold, design: .serif)
        case "pre":
            return .system(size: size - 2, design: .monospaced)
        default:
            return .system(size: size, design: .serif)
        }
    }
}

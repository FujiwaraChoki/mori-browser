import SwiftUI

struct SpaceEditor: View {
    var store: BrowserStore
    let spaceID: UUID
    @Environment(\.dismiss) private var dismiss
    @Environment(\.moriPalette) private var palette
    @State private var name: String
    @State private var confirmDelete = false

    init(store: BrowserStore, spaceID: UUID) {
        self.store = store
        self.spaceID = spaceID
        _name = State(initialValue: store.workspace.spaces.first { $0.id == spaceID }?.name ?? "")
    }

    private var space: Space? {
        store.workspace.spaces.first { $0.id == spaceID }
    }

    var body: some View {
        Form {
            Section("Name") {
                TextField("Name", text: $name)
                    .onChange(of: name) { _, value in
                        store.rename(space: spaceID, to: value)
                    }
            }
            Section("Symbol") {
                symbolGrid(SpaceSymbols.choices)
                symbolGrid(SpaceSymbols.emoji)
            }
            Section("Theme") {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 10) {
                        themeButton(nil, name: "None")
                        ForEach(ThemePreset.all) { preset in
                            themeButton(preset.theme, name: preset.name)
                        }
                    }
                    .padding(.vertical, 4)
                }
            }
            if let space, store.canDelete(space) {
                Section {
                    Button("Delete Space", role: .destructive) { confirmDelete = true }
                }
            }
        }
        .navigationTitle("Edit Space")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        .confirmationDialog("Delete this space?", isPresented: $confirmDelete, titleVisibility: .visible) {
            Button("Delete Space", role: .destructive) {
                store.delete(space: spaceID)
                dismiss()
            }
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("Tabs in this space will close. Favorites stay.")
        }
    }

    private func symbolGrid(_ symbols: [String]) -> some View {
        LazyVGrid(columns: Array(repeating: GridItem(.flexible(), spacing: 8), count: 6), spacing: 8) {
            ForEach(symbols, id: \.self) { symbol in
                Button {
                    store.setSymbol(symbol, for: spaceID)
                } label: {
                    SymbolIcon(name: symbol, size: 16)
                        .frame(width: 36, height: 36)
                        .background(
                            RoundedRectangle(cornerRadius: 8, style: .continuous)
                                .fill(space?.symbol == symbol ? palette.selected : Color.clear)
                        )
                }
                .buttonStyle(.plain)
                .accessibilityLabel(symbol)
            }
        }
    }

    private func themeButton(_ theme: GradientTheme?, name: String) -> some View {
        let selected = space?.theme?.presetID == theme?.presetID && (theme != nil || space?.theme == nil)
        return Button {
            store.setTheme(theme, for: spaceID)
        } label: {
            VStack(spacing: 6) {
                Circle()
                    .fill(swatch(theme))
                    .frame(width: 44, height: 44)
                    .overlay {
                        Circle().strokeBorder(selected ? palette.foreground : Color.clear, lineWidth: 2)
                    }
                Text(name)
                    .font(.caption2)
                    .lineLimit(1)
                    .frame(width: 64)
            }
        }
        .buttonStyle(.plain)
        .accessibilityLabel(name)
    }

    private func swatch(_ theme: GradientTheme?) -> LinearGradient {
        let colors = theme?.colors.map { Color(hex: $0) } ?? [palette.sidebar, palette.tile]
        return LinearGradient(colors: colors, startPoint: .topLeading, endPoint: .bottomTrailing)
    }
}

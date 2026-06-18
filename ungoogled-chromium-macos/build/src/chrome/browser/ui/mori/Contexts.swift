import SwiftUI

// MARK: - Glyph grid

/// The shared icon grid (Arc's folder-icon picker layout): 8 columns of fill
/// glyphs that tint to the sidebar foreground. Used by the folder icon picker,
/// the context editor, and the create-context flow.
struct GlyphGrid: View {
    var selected: String?
    var glyphSize: CGFloat = 15
    let onPick: (String) -> Void

    @Environment(\.palette) private var p

    private let columns = Array(repeating: GridItem(.flexible(), spacing: 2), count: 8)

    var body: some View {
        LazyVGrid(columns: columns, spacing: 2) {
            ForEach(GlyphLibrary.all) { glyph in
                GlyphCell(glyph: glyph,
                          isSelected: glyph.asset == selected,
                          glyphSize: glyphSize) {
                    onPick(glyph.asset)
                }
            }
        }
    }
}

private struct GlyphCell: View {
    let glyph: GlyphLibrary.Glyph
    let isSelected: Bool
    let glyphSize: CGFloat
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Icon(name: glyph.asset, size: glyphSize)
                .foregroundStyle(p.foreground.color.opacity(hovering || isSelected ? 1 : 0.78))
                .frame(width: glyphSize + 13, height: glyphSize + 13)
                .background(
                    RoundedRectangle(cornerRadius: Radius.md + 2, style: .continuous)
                        .fill(isSelected
                              ? p.primary.color.opacity(0.22)
                              : hovering ? p.foreground.color.opacity(0.08) : .clear)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(glyph.label)
        .onHover { hovering = $0 }
    }
}

// MARK: - Folder icon picker

/// The Arc-style popover for customizing a folder's icon: an "Icon" header
/// with a trash button that resets to the plain folder, above the glyph grid.
struct FolderIconPicker: View {
    @ObservedObject var store: BrowserStore
    let folder: TabFolder
    @Binding var isPresented: Bool

    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 8) {
            HStack(spacing: 10) {
                Spacer()
                Text("Icon")
                    .font(Typography.ui(Typography.base, weight: .semibold))
                    .foregroundStyle(p.foreground.color)
                IconButton(systemName: "trash", size: 24) {
                    store.setFolderSymbol(folder.id, symbol: "folder")
                    isPresented = false
                }
                .help("Remove custom icon")
                Spacer()
            }
            GlyphGrid(selected: GlyphLibrary.isGlyph(folder.symbol) ? folder.symbol : nil) { asset in
                store.setFolderSymbol(folder.id, symbol: asset)
                isPresented = false
            }
        }
        .padding(10)
    }
}

// MARK: - Bottom-bar context switcher

/// The row of context glyphs in the sidebar's bottom bar — Arc's space
/// switcher. The active context keeps its full color; the others are dimmed
/// glyphs that switch on click.
struct ContextSwitcherStrip: View {
    @ObservedObject var store: BrowserStore

    var body: some View {
        HStack(spacing: 4) {
            ForEach(Array(store.contexts.enumerated()), id: \.element.id) { index, context in
                ContextChip(store: store,
                            context: context,
                            isActive: context.id == store.activeContextID,
                            ordinal: index < 9 ? index + 1 : nil)
            }
        }
        .animation(Motion.snappy, value: store.activeContextID)
    }
}

private struct ContextChip: View {
    @ObservedObject var store: BrowserStore
    let context: BrowserContext
    let isActive: Bool
    /// 1-based switcher slot, when within the Ctrl-1…Ctrl-9 range.
    var ordinal: Int? = nil

    @Environment(\.palette) private var p
    @State private var hovering = false
    @State private var showEditor = false

    var body: some View {
        Button {
            if isActive {
                showEditor = true
            } else {
                store.switchContext(to: context.id)
            }
        } label: {
            Icon(name: context.symbol, size: isActive ? 15 : 13)
                .foregroundStyle(p.sidebarForeground.color
                    .opacity(isActive ? 1 : hovering ? 0.75 : 0.45))
                .frame(width: 30, height: 30)
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(!isActive && hovering ? p.sidebarForeground.color.opacity(0.08) : .clear)
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .help(ordinal.map { "\(context.name) (⌃\($0))" } ?? context.name)
        .onHover { hovering = $0 }
        .contextMenu {
            Button("Edit Context…") {
                if !isActive { store.switchContext(to: context.id) }
                showEditor = true
            }
            Divider()
            Button("Delete Context", role: .destructive) {
                store.deleteContext(context.id)
            }
            .disabled(store.contexts.count <= 1)
        }
        .popover(isPresented: $showEditor, arrowEdge: .bottom) {
            ContextEditor(store: store, contextID: context.id)
                .environment(\.palette, p)
        }
    }
}

/// Per-context settings in one popover: rename, glyph, and theme.
private struct ContextEditor: View {
    @ObservedObject var store: BrowserStore
    let contextID: BrowserContext.ID

    @Environment(\.palette) private var p
    @State private var draftName = ""

    private var context: BrowserContext? {
        store.contexts.first { $0.id == contextID }
    }

    var body: some View {
        if let context {
            VStack(alignment: .leading, spacing: 12) {
                TextField("Context name", text: $draftName)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .padding(.horizontal, 10)
                    .frame(height: 30)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                            .fill(p.foreground.color.opacity(0.06))
                    )
                    .onSubmit(commitRename)
                    .onAppear { draftName = context.name }

                GlyphGrid(selected: context.symbol) { asset in
                    store.setContextSymbol(contextID, symbol: asset)
                }

                Divider()

                Text("Theme")
                    .font(Typography.ui(Typography.label, weight: .medium))
                    .foregroundStyle(p.mutedForeground.color)
                CompactThemeStrip(
                    selected: context.theme,
                    onPick: { store.setContextTheme(contextID, theme: $0) })
            }
            .padding(12)
            .frame(width: 248)
            .onDisappear(perform: commitRename)
        }
    }

    private func commitRename() {
        guard let context, draftName != context.name else { return }
        store.renameContext(contextID, to: draftName)
    }
}

// MARK: - Compact theme strip

/// A one-popover-friendly theme picker: the "no theme" chip, the curated
/// gradient presets as round swatches, then the solid palette.
struct CompactThemeStrip: View {
    let selected: GradientTheme
    let onPick: (GradientTheme) -> Void

    @Environment(\.palette) private var p

    private let columns = [GridItem(.adaptive(minimum: 30), spacing: 8, alignment: .leading)]

    var body: some View {
        LazyVGrid(columns: columns, alignment: .leading, spacing: 8) {
            ThemeDot(isSelected: selected.isEmpty) {
                Circle()
                    .fill(LinearGradient(colors: [p.card.color, p.sidebar.color],
                                         startPoint: .topLeading, endPoint: .bottomTrailing))
            } action: {
                onPick(.none)
            }
            .help("Default")

            ForEach(ThemePreset.all) { preset in
                ThemeDot(isSelected: selected.presetID == preset.id) {
                    GradientMesh(colors: preset.theme.dots.map(\.rgb.color),
                                 relativeBlur: 0.5, maxBlur: 10)
                        .clipShape(Circle())
                } action: {
                    onPick(preset.theme)
                }
                .help(preset.name)
            }

            ForEach(SolidPalette.swatches, id: \.self) { hex in
                ThemeDot(isSelected: selected.solidHex?.caseInsensitiveCompare(hex) == .orderedSame) {
                    Circle().fill(TokenColor(hex: hex).color)
                } action: {
                    onPick(.solid(RGB(TokenColor(hex: hex))))
                }
                .help(hex)
            }
        }
    }
}

private struct ThemeDot<Swatch: View>: View {
    let isSelected: Bool
    @ViewBuilder let swatch: () -> Swatch
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            swatch()
                .frame(width: 26, height: 26)
                .overlay(Circle().strokeBorder(.white.opacity(0.15), lineWidth: 1))
                .overlay {
                    Circle()
                        .strokeBorder(p.primary.color, lineWidth: 2)
                        .padding(-3)
                        .opacity(isSelected ? 1 : 0)
                }
                .scaleEffect(hovering ? 1.08 : 1)
                .contentShape(Circle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: isSelected)
        .animation(Motion.snappy, value: hovering)
    }
}

// MARK: - Plus menu

/// The bottom bar's "+" affordance: an Arc-style popover offering New Tab,
/// New Split, and New Context.
struct PlusMenuButton: View {
    @ObservedObject var store: BrowserStore

    @Environment(\.palette) private var p
    @State private var showMenu = false

    var body: some View {
        IconButton(systemName: showMenu ? "xmark" : "plus", size: 30) {
            showMenu.toggle()
        }
        .help("New tab, split, or context")
        .popover(isPresented: $showMenu, arrowEdge: .top) {
            VStack(alignment: .leading, spacing: 2) {
                PlusMenuRow(icon: "square.stack", title: "New Context") {
                    showMenu = false
                    store.contextCreationVisible = true
                }

                Divider().padding(.vertical, 4)

                PlusMenuRow(icon: "rectangle.split.2x1", title: "New Split",
                            shortcut: "⌃⇧=",
                            disabled: store.selectedTab == nil) {
                    showMenu = false
                    store.newSplit()
                }
                PlusMenuRow(icon: "plus", title: "New Tab", shortcut: "⌘T") {
                    showMenu = false
                    store.presentLauncher()
                }
            }
            .padding(8)
            .frame(width: 224)
            .environment(\.palette, p)
        }
    }
}

private struct PlusMenuRow: View {
    let icon: String
    let title: String
    var shortcut: String? = nil
    var disabled: Bool = false
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            HStack(spacing: 10) {
                Icon(name: icon, size: 15)
                    .foregroundStyle(p.foreground.color.opacity(0.85))
                    .frame(width: 18)
                Text(title)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                Spacer()
                if let shortcut {
                    Text(shortcut)
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                }
            }
            .padding(.horizontal, 10)
            .frame(height: 32)
            .background(
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .fill(hovering ? p.foreground.color.opacity(0.07) : .clear)
            )
            .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .disabled(disabled)
        .opacity(disabled ? 0.4 : 1)
        .onHover { hovering = $0 }
    }
}

// MARK: - Create a Context

/// The Arc-style "Create a Space" flow, hosted inside the sidebar in place of
/// the tab list: stacked-card art, a name field, an icon, a theme, and a big
/// primary Create button with Cancel underneath.
struct CreateContextView: View {
    @ObservedObject var store: BrowserStore

    @Environment(\.palette) private var p
    @State private var name = ""
    @State private var symbol = "glyph-circle"
    @State private var theme: GradientTheme = .none
    @State private var themeExpanded = false
    @FocusState private var nameFocused: Bool

    var body: some View {
        VStack(spacing: 0) {
            ScrollView {
                VStack(spacing: 14) {
                    StackedCardsArt()
                        .padding(.top, 26)

                    VStack(spacing: 6) {
                        Text("Create a Context")
                            .font(Typography.ui(17, weight: .semibold))
                            .foregroundStyle(p.sidebarForeground.color)
                        Text("Separate your tabs for life,\nwork, projects, and more.")
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.mutedForeground.color)
                            .multilineTextAlignment(.center)
                    }
                    .padding(.bottom, 8)

                    // Name field.
                    HStack(spacing: 8) {
                        Icon(name: symbol, size: 14)
                            .foregroundStyle(p.mutedForeground.color)
                        TextField("Context name...", text: $name)
                            .textFieldStyle(.plain)
                            .font(Typography.ui(Typography.base, weight: .medium))
                            .foregroundStyle(p.sidebarForeground.color)
                            .focused($nameFocused)
                            .onSubmit(create)
                    }
                    .padding(.horizontal, 12)
                    .frame(height: 38)
                    .background(fieldBackground)

                    // Icon picker.
                    VStack(alignment: .leading, spacing: 8) {
                        Text("Icon")
                            .font(Typography.ui(Typography.label, weight: .medium))
                            .foregroundStyle(p.mutedForeground.color)
                        GlyphGrid(selected: symbol, glyphSize: 14) { symbol = $0 }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(fieldBackground)

                    // Theme row, expanding in place like Arc's.
                    VStack(alignment: .leading, spacing: 10) {
                        Button {
                            withAnimation(Motion.snappy) { themeExpanded.toggle() }
                        } label: {
                            HStack(spacing: 8) {
                                Icon(name: "circle.lefthalf.filled", size: 14)
                                    .foregroundStyle(p.mutedForeground.color)
                                Text("Choose a Theme")
                                    .font(Typography.ui(Typography.base, weight: .medium))
                                    .foregroundStyle(p.sidebarForeground.color)
                                Spacer()
                                Icon(name: themeExpanded ? "chevron.up" : "chevron.down", size: 11)
                                    .foregroundStyle(p.mutedForeground.color)
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)

                        if themeExpanded {
                            CompactThemeStrip(selected: theme) { theme = $0 }
                        }
                    }
                    .padding(12)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .background(fieldBackground)
                }
                .padding(.horizontal, 12)
            }

            Spacer(minLength: 0)

            VStack(spacing: 10) {
                Button(action: create) {
                    Text("Create Context")
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(p.primaryForeground.color)
                        .frame(maxWidth: .infinity)
                        .frame(height: 38)
                        .background(
                            RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                                .fill(p.primary.color)
                        )
                        .contentShape(Rectangle())
                }
                .buttonStyle(PressShrinkButtonStyle())

                Button("Cancel") {
                    withAnimation(Motion.reveal) { store.contextCreationVisible = false }
                }
                .buttonStyle(.plain)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.mutedForeground.color)
            }
            .padding(.horizontal, 12)
            .padding(.bottom, 14)
        }
        .onAppear {
            DispatchQueue.main.async { nameFocused = true }
        }
    }

    private var fieldBackground: some View {
        RoundedRectangle(cornerRadius: Radius.xl, style: .continuous)
            .fill(p.sidebarForeground.color.opacity(0.06))
    }

    private func create() {
        store.addContext(name: name, symbol: symbol, theme: theme)
        withAnimation(Motion.reveal) { store.contextCreationVisible = false }
        // A fresh context starts empty; greet it with the launcher so the
        // first tab is one keystroke away.
        store.presentLauncher()
    }
}

/// Three fanned "space cards" echoing Arc's create-space artwork, drawn from
/// theme colors so the art matches the chrome.
private struct StackedCardsArt: View {
    @Environment(\.palette) private var p

    var body: some View {
        ZStack {
            card(rotation: -10, offset: CGSize(width: -22, height: 4),
                 glyph: "glyph-leaf")
            card(rotation: 10, offset: CGSize(width: 22, height: 4),
                 glyph: "glyph-bolt")
            card(rotation: 0, offset: .zero, glyph: "glyph-star")
        }
        .frame(height: 74)
    }

    private func card(rotation: Double, offset: CGSize, glyph: String) -> some View {
        RoundedRectangle(cornerRadius: 8, style: .continuous)
            .fill(p.card.color)
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.7), lineWidth: 1)
            )
            .overlay(
                Icon(name: glyph, size: 18)
                    .foregroundStyle(p.primary.color.opacity(0.85))
            )
            .frame(width: 52, height: 64)
            .shadow(color: .black.opacity(0.18), radius: 5, y: 2)
            .rotationEffect(.degrees(rotation))
            .offset(offset)
    }
}

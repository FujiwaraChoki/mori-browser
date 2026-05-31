import SwiftUI

/// The preferences sheet. Styled to the Mori design system: quiet labels,
/// token colors, rounded-xl surfaces, segmented appearance control.
struct SettingsView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared
    @ObservedObject private var extensions = ExtensionStore.shared
    @Environment(\.palette) private var p

    var body: some View {
        VStack(spacing: 0) {
            header
            Hairline().opacity(0.6)
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    generalSection
                    searchSection
                    appearanceSection
                    mediaSection
                    extensionsSection
                    aboutSection
                }
                .padding(20)
            }
        }
        .frame(width: 540, height: 560)
        .background(p.background.color)
    }

    private var header: some View {
        HStack {
            Text("Settings")
                .font(Typography.ui(16, weight: .semibold))
                .foregroundStyle(p.foreground.color)
            Spacer()
            Button {
                store.settingsVisible = false
            } label: {
                Text("Done")
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.primaryForeground.color)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 6)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                            .fill(p.primary.color)
                    )
            }
            .buttonStyle(.plain)
            .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 18)
        .frame(height: 56)
    }

    // MARK: Sections

    private var aboutSection: some View {
        Section(title: "About") {
            HStack(alignment: .center, spacing: 14) {
                Icon(name: "mori", size: 40)
                    .foregroundStyle(p.primary.color)
                VStack(alignment: .leading, spacing: 4) {
                    Text("Mori")
                        .font(Typography.ui(15, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                    Text("A native macOS browser powered by Chromium (CEF).")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color)
                    Text("Version 0.1.0")
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                }
                Spacer(minLength: 0)
            }
        }
    }

    private var generalSection: some View {
        Section(title: "General") {
            Field(label: "Homepage") {
                SettingTextField(text: $settings.homepageURL, placeholder: "https://…")
            }
            Field(label: "New tab opens") {
                EnumMenu(selection: $settings.newTabBehavior,
                         options: NewTabBehavior.allCases) { $0.label }
            }
        }
    }

    private var searchSection: some View {
        Section(title: "Search") {
            Field(label: "Search engine") {
                EnumMenu(selection: $settings.searchEngine,
                         options: SearchEngine.allCases) { $0.label }
            }
            if settings.searchEngine == .custom {
                Field(label: "Custom URL") {
                    SettingTextField(text: $settings.customSearchTemplate,
                                     placeholder: "https://example.com/?q={query}")
                }
                Text("Use {query} where the search terms should go.")
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.mutedForeground.color)
            }
        }
    }

    private var appearanceSection: some View {
        Section(title: "Appearance") {
            Field(label: "Theme") {
                SegmentedTheme(selection: $settings.theme)
            }
            Field(label: "Sidebar side") {
                EnumMenu(selection: $settings.sidebarPosition,
                         options: SidebarPosition.allCases) { $0.label }
            }
            ToggleRow(isOn: $settings.showSidebarOnLaunch) {
                Text("Show tab sidebar on launch")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
            }

            Hairline().opacity(0.5)

            VStack(alignment: .leading, spacing: 4) {
                Text("Color theme")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                Text("Pick an anime-inspired theme to wash the chrome and accent.")
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.mutedForeground.color)
            }
            ThemePicker()
                .frame(maxWidth: .infinity, alignment: .center)
        }
    }

    private var mediaSection: some View {
        Section(title: "Media") {
            ToggleRow(isOn: $settings.autoPiP) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("Automatic Picture in Picture")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.foreground.color)
                    Text("Pop a playing video out when you switch tabs.")
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                }
            }
        }
    }

    private var extensionsSection: some View {
        Section(title: "Extensions") {
            if extensions.extensions.isEmpty {
                Text("No extensions installed.")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.mutedForeground.color)
            } else {
                VStack(spacing: 0) {
                    ForEach(extensions.extensions) { ext in
                        ExtensionRow(ext: ext, store: extensions)
                        if ext.id != extensions.extensions.last?.id {
                            Hairline().opacity(0.5)
                        }
                    }
                }
            }

            if let error = extensions.lastError {
                Text(error)
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.destructive.color)
            }

            HStack(spacing: 10) {
                Button {
                    extensions.presentImportPanel()
                } label: {
                    HStack(spacing: 5) {
                        Icon(name: "plus", size: 12, weight: .semibold)
                        Text("Add Extension…")
                            .font(Typography.ui(Typography.base, weight: .medium))
                    }
                    .foregroundStyle(p.foreground.color)
                    .padding(.horizontal, 12)
                    .frame(height: 30)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .fill(p.input.color.opacity(0.5))
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
                    )
                }
                .buttonStyle(.plain)
            }

            Text("Load unpacked Chrome extensions (the folder containing manifest.json). Mori runs supported content scripts, extension pages, popups, and background/event scripts inside its own Chromium engine; the full chrome.* API surface is still in progress.")
                .font(Typography.ui(Typography.label))
                .foregroundStyle(p.mutedForeground.color)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

// MARK: - Extension row

private struct ExtensionRow: View {
    let ext: BrowserExtension
    @ObservedObject var store: ExtensionStore
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 11) {
            icon
            VStack(alignment: .leading, spacing: 2) {
                HStack(spacing: 6) {
                    Text(ext.name)
                        .font(Typography.ui(Typography.base, weight: .medium))
                        .foregroundStyle(p.foreground.color)
                        .lineLimit(1)
                    if !ext.version.isEmpty {
                        Text("v\(ext.version)")
                            .font(Typography.ui(Typography.small))
                            .foregroundStyle(p.mutedForeground.color)
                    }
                }
                if !ext.detail.isEmpty {
                    Text(ext.detail)
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                        .lineLimit(2)
                }
            }
            Spacer(minLength: 8)

            Toggle("", isOn: Binding(
                get: { ext.enabled },
                set: { store.setEnabled(ext, $0) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .tint(p.primary.color)

            Button {
                store.remove(ext)
            } label: {
                Icon(name: "trash", size: 14, weight: .regular)
                    .foregroundStyle(p.mutedForeground.color)
            }
            .buttonStyle(.plain)
            .help("Remove extension")
        }
        .padding(.vertical, 10)
    }

    @ViewBuilder private var icon: some View {
        if let path = ext.iconPath, let image = NSImage(contentsOfFile: path) {
            Image(nsImage: image)
                .resizable()
                .interpolation(.high)
                .frame(width: 28, height: 28)
                .clipShape(RoundedRectangle(cornerRadius: Radius.sm, style: .continuous))
        } else {
            RoundedRectangle(cornerRadius: Radius.sm, style: .continuous)
                .fill(p.input.color.opacity(0.6))
                .frame(width: 28, height: 28)
                .overlay(
                    Icon(name: "puzzlepiece.extension", size: 16, weight: .regular)
                        .foregroundStyle(p.mutedForeground.color)
                )
        }
    }
}

// MARK: - Building blocks

private struct Section<Content: View>: View {
    let title: String
    @ViewBuilder var content: Content
    @Environment(\.palette) private var p

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title.uppercased())
                .font(Typography.ui(Typography.small, weight: .medium))
                .foregroundStyle(p.mutedForeground.color)
                .tracking(0.4)
            VStack(alignment: .leading, spacing: 14) {
                content
            }
            .padding(16)
            .frame(maxWidth: .infinity, alignment: .leading)
            .background(
                RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                    .fill(p.card.color.opacity(0.6))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
            )
        }
    }
}

private struct Field<Content: View>: View {
    let label: String
    @ViewBuilder var content: Content
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 14) {
            Text(label)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
                .frame(width: 120, alignment: .leading)
            content
                .frame(maxWidth: .infinity, alignment: .trailing)
        }
    }
}

private struct ToggleRow<Label: View>: View {
    @Binding var isOn: Bool
    @ViewBuilder var label: Label
    @Environment(\.palette) private var p

    var body: some View {
        HStack(alignment: .center, spacing: 14) {
            label
                .frame(maxWidth: .infinity, alignment: .leading)
            Toggle("", isOn: $isOn)
                .labelsHidden()
                .toggleStyle(.switch)
                .tint(p.primary.color)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

private struct SettingTextField: View {
    @Binding var text: String
    let placeholder: String
    @Environment(\.palette) private var p

    var body: some View {
        TextField(placeholder, text: $text)
            .textFieldStyle(.plain)
            .font(Typography.ui(Typography.base))
            .foregroundStyle(p.foreground.color)
            .padding(.horizontal, 10)
            .frame(height: 30)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(p.input.color.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
            )
    }
}

/// A dropdown driven by a `CaseIterable` enum, styled like a Mori select.
private struct EnumMenu<T: Hashable & Identifiable>: View {
    @Binding var selection: T
    let options: [T]
    let label: (T) -> String
    @Environment(\.palette) private var p

    var body: some View {
        Menu {
            ForEach(options) { option in
                Button(label(option)) { selection = option }
            }
        } label: {
            HStack(spacing: 6) {
                Text(label(selection))
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                Icon(name: "chevron.up.chevron.down", size: 12)
                    .foregroundStyle(p.mutedForeground.color)
            }
            .padding(.horizontal, 11)
            .frame(height: 30)
            .background(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .fill(p.input.color.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.6), lineWidth: 1)
            )
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .fixedSize()
    }
}

/// Three-up segmented control for the theme preference.
private struct SegmentedTheme: View {
    @Binding var selection: ThemePreference
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 3) {
            ForEach(ThemePreference.allCases) { option in
                let active = option == selection
                Button {
                    withAnimation(Motion.state) { selection = option }
                } label: {
                    HStack(spacing: 5) {
                        Icon(name: option.symbol, size: 13)
                        Text(option.label)
                            .font(Typography.ui(Typography.label))
                    }
                    .foregroundStyle(active ? p.foreground.color : p.mutedForeground.color)
                    .padding(.horizontal, 12)
                    .frame(height: 26)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.sm + 2, style: .continuous)
                            .fill(active ? p.background.color : .clear)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: Radius.sm + 2, style: .continuous)
                            .strokeBorder(active ? p.border.color.opacity(0.7) : .clear, lineWidth: 1)
                    )
                }
                .buttonStyle(.plain)
            }
        }
        .padding(3)
        .background(
            RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                .fill(p.input.color.opacity(0.5))
        )
    }
}

import SwiftUI

/// The preferences page. Rendered full-bleed inside the browser card (not a
/// modal sheet). Styled to the Mori design system: quiet labels, token colors,
/// rounded-xl surfaces, segmented appearance control. The scrolling content is
/// centered and width-constrained so it stays readable on a wide window.
struct SettingsView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var settings = BrowserSettings.shared
    @ObservedObject private var extensions = ExtensionStore.shared
    @ObservedObject private var boosts = BoostStore.shared
    @Environment(\.palette) private var p

    /// Comfortable reading column for the settings content.
    private let contentWidth: CGFloat = 560

    var body: some View {
        VStack(spacing: 0) {
            header
            Hairline().opacity(0.6)
            ScrollView {
                VStack(alignment: .leading, spacing: 22) {
                    generalSection
                    searchSection
                    aiSection
                    appearanceSection
                    tabsSection
                    RoutingSection(store: store)
                    boostsSection
                    mediaSection
                    extensionsSection
                    aboutSection
                }
                .frame(maxWidth: contentWidth)
                .padding(.horizontal, 24)
                .padding(.vertical, 28)
                .frame(maxWidth: .infinity)
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(p.background.color)
    }

    private var header: some View {
        HStack(spacing: 12) {
            Button {
                store.settingsVisible = false
            } label: {
                HStack(spacing: 5) {
                    Icon(name: "chevron.left", size: 13, weight: .semibold)
                    Text("Back")
                        .font(Typography.ui(Typography.base, weight: .medium))
                }
                .foregroundStyle(p.foreground.color)
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
            .buttonStyle(.plain)
            .help("Back to browsing")
            .accessibilityLabel("Back to browsing")

            Text("Settings")
                .font(Typography.ui(Typography.title, weight: .semibold))
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
                        .font(Typography.ui(Typography.title, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                    Text("A native macOS browser powered by Chromium (CEF).")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color)
                    Text("Version 2.0.5")
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                        .textSelection(.enabled)
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

    private var aiSection: some View {
        Section(title: "AI") {
            ToggleRow(isOn: $settings.aiIntegrationEnabled) {
                VStack(alignment: .leading, spacing: 2) {
                    Text("AI integration")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.foreground.color)
                    Text("Allow Mori to use the local Codex assistant and browser automation tools.")
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                        .fixedSize(horizontal: false, vertical: true)
                }
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
                Text("Pick a preset theme to wash the chrome and accent.")
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.mutedForeground.color)
            }
            ThemeList()
            ThemeIntensityControls()

            VStack(alignment: .leading, spacing: 8) {
                Text("Solid color")
                    .font(Typography.ui(Typography.base))
                    .foregroundStyle(p.foreground.color)
                Text("Or wash the chrome in a single flat color.")
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.mutedForeground.color)
                SolidThemeSwatches()
                    .padding(.top, 2)
            }
        }
    }

    private var boostsSection: some View {
        Section(title: "Boosts") {
            if boosts.boosts.isEmpty {
                HStack(spacing: 8) {
                    Icon(name: "wand.and.stars", size: 15, weight: .light)
                        .foregroundStyle(p.mutedForeground.color)
                    Text("No site Boosts saved")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 4)
            } else {
                VStack(spacing: 0) {
                    ForEach(boosts.boosts) { boost in
                        BoostSettingsRow(boost: boost, store: store, boosts: boosts)
                        if boost.id != boosts.boosts.last?.id {
                            Hairline().opacity(0.5)
                        }
                    }
                }
            }
        }
    }

    private var tabsSection: some View {
        Section(title: "Tabs") {
            Field(label: "Sleep idle tabs") {
                IntMenu(value: $settings.autoSleepMinutes, options: Self.sleepOptions)
            }
            Field(label: "Archive idle tabs") {
                IntMenu(value: $settings.autoArchiveHours, options: Self.archiveOptions)
            }
            Text("Sleeping frees a background tab's memory; it reloads when you return. "
                 + "Archiving closes stale tabs to the restorable Archive in your Library.")
                .font(Typography.ui(Typography.label))
                .foregroundStyle(p.mutedForeground.color)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private static let sleepOptions: [(Int, String)] = [
        (0, "Never"), (15, "15 minutes"), (30, "30 minutes"),
        (60, "1 hour"), (180, "3 hours"), (360, "6 hours")
    ]
    private static let archiveOptions: [(Int, String)] = [
        (0, "Never"), (12, "12 hours"), (24, "1 day"),
        (72, "3 days"), (168, "1 week"), (720, "30 days")
    ]

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
            if let error = extensions.lastError {
                Text(error)
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.destructive.color)
                    .textSelection(.enabled)
            }

            if extensions.extensions.isEmpty {
                HStack(spacing: 8) {
                    Icon(name: "puzzlepiece.extension", size: 15, weight: .light)
                        .foregroundStyle(p.mutedForeground.color)
                    Text("No extensions installed")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.vertical, 4)
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

            HStack(spacing: 10) {
                Button {
                    extensions.presentImportPanel()
                } label: {
                    HStack(spacing: 5) {
                        Icon(name: "plus", size: 12, weight: .semibold)
                        Text("Load Unpacked…")
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

                Button {
                    extensions.openManagePage()
                } label: {
                    HStack(spacing: 5) {
                        Icon(name: "puzzlepiece.extension", size: 12, weight: .regular)
                        Text("Manage…")
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

            Text("Extensions are installed and managed by Chrome's native extension service.")
                .font(Typography.ui(Typography.label))
                .foregroundStyle(p.mutedForeground.color)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

// MARK: - Settings rows

private struct BoostSettingsRow: View {
    let boost: SiteBoost
    @ObservedObject var store: BrowserStore
    @ObservedObject var boosts: BoostStore
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 11) {
            Icon(name: "wand.and.stars", size: 15, weight: .regular)
                .foregroundStyle(p.mutedForeground.color)
                .frame(width: 28, height: 28)
                .background(Circle().fill(p.input.color.opacity(0.5)))

            VStack(alignment: .leading, spacing: 2) {
                Text(boost.host)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                Text(summary)
                    .font(Typography.ui(Typography.label))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
            }

            Spacer(minLength: 8)

            Button {
                visitSite()
            } label: {
                Text("Visit")
                    .font(Typography.ui(Typography.label, weight: .medium))
                    .foregroundStyle(p.foreground.color)
                    .padding(.horizontal, 9)
                    .frame(height: 26)
                    .background(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .fill(p.input.color.opacity(0.5))
                    )
            }
            .buttonStyle(.plain)
            .help("Visit site")

            Toggle("", isOn: Binding(
                get: { boosts.boost(forHost: boost.host)?.enabled ?? boost.enabled },
                set: { setEnabled($0) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .controlSize(.mini)
            .tint(p.primary.color)

            Button {
                boosts.remove(host: boost.host)
                reloadMatchingTabs()
            } label: {
                Icon(name: "trash", size: 13, weight: .regular)
                    .foregroundStyle(p.mutedForeground.color)
            }
            .buttonStyle(.plain)
            .help("Delete Boost")
        }
        .padding(.vertical, 10)
    }

    private var summary: String {
        var parts: [String] = []
        if !boost.css.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            parts.append("CSS")
        }
        if !boost.js.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            parts.append("JS")
        }
        if !boost.zappedSelectors.isEmpty {
            parts.append("\(boost.zappedSelectors.count) zap\(boost.zappedSelectors.count == 1 ? "" : "s")")
        }
        return parts.isEmpty ? "Empty Boost" : parts.joined(separator: " · ")
    }

    private func setEnabled(_ enabled: Bool) {
        boosts.setEnabled(enabled, host: boost.host)
        reloadMatchingTabs()
    }

    private func visitSite() {
        store.newTab(url: "https://\(boost.host)", select: true)
    }

    private func reloadMatchingTabs() {
        for tab in store.tabs where matches(tab.urlString) {
            tab.reload()
        }
    }

    private func matches(_ url: String) -> Bool {
        guard let host = URLComponents(string: url)?.host?.lowercased() else { return false }
        return host == boost.host || host.hasSuffix("." + boost.host)
    }
}

private struct ExtensionRow: View {
    let ext: ChromeExtensionInfo
    @ObservedObject var store: ExtensionStore
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 11) {
            ExtensionIconView(ext: ext, size: 28)
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
                    if ext.installType == "development" {
                        Text("Unpacked")
                            .font(Typography.ui(Typography.small))
                            .foregroundStyle(p.mutedForeground.color)
                            .padding(.horizontal, 5)
                            .padding(.vertical, 1)
                            .background(Capsule().fill(p.input.color.opacity(0.6)))
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

            if ext.hasOptionsPage {
                Button {
                    store.openOptions(ext)
                } label: {
                    Icon(name: "slider.horizontal.3", size: 14, weight: .regular)
                        .foregroundStyle(p.mutedForeground.color)
                }
                .buttonStyle(.plain)
                .help("Extension options")
            }

            Toggle("", isOn: Binding(
                get: { ext.enabled },
                set: { store.setEnabled(ext, $0) }
            ))
            .labelsHidden()
            .toggleStyle(.switch)
            .tint(p.primary.color)
            .disabled(!ext.mayDisable)

            Button {
                store.remove(ext)
            } label: {
                Icon(name: "trash", size: 14, weight: .regular)
                    .foregroundStyle(p.mutedForeground.color)
            }
            .buttonStyle(.plain)
            .disabled(!ext.mayDisable)
            .help("Remove extension")
        }
        .padding(.vertical, 10)
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

/// A dropdown over preset integer values (used for sleep/archive intervals).
private struct IntMenu: View {
    @Binding var value: Int
    let options: [(Int, String)]
    @Environment(\.palette) private var p

    private var label: String {
        options.first { $0.0 == value }?.1 ?? "\(value)"
    }

    var body: some View {
        Menu {
            ForEach(options, id: \.0) { option in
                Button(option.1) { value = option.0 }
            }
        } label: {
            HStack(spacing: 6) {
                Text(label)
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

/// Air Traffic Control: routing rules that send hosts to a chosen space.
private struct RoutingSection: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var routes = RouteStore.shared
    @Environment(\.palette) private var p
    @State private var newPattern = ""
    @State private var newContextID: BrowserContext.ID?

    var body: some View {
        Section(title: "Air Traffic Control") {
            Text("Open matching sites in a chosen space automatically.")
                .font(Typography.ui(Typography.label))
                .foregroundStyle(p.mutedForeground.color)

            if !routes.rules.isEmpty {
                VStack(spacing: 0) {
                    ForEach(routes.rules) { rule in
                        ruleRow(rule)
                        if rule.id != routes.rules.last?.id {
                            Hairline().opacity(0.5)
                        }
                    }
                }
            }

            HStack(spacing: 8) {
                SettingTextField(text: $newPattern, placeholder: "figma.com")
                Menu {
                    ForEach(store.contexts) { context in
                        Button(context.name) { newContextID = context.id }
                    }
                } label: {
                    HStack(spacing: 5) {
                        Text(selectedContextName)
                            .font(Typography.ui(Typography.base))
                            .foregroundStyle(p.foreground.color)
                            .lineLimit(1)
                        Icon(name: "chevron.up.chevron.down", size: 11)
                            .foregroundStyle(p.mutedForeground.color)
                    }
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
                .menuStyle(.borderlessButton)
                .menuIndicator(.hidden)
                .fixedSize()

                Button { addRule() } label: {
                    Text("Add")
                        .font(Typography.ui(Typography.base, weight: .medium))
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
                .disabled(newPattern.trimmingCharacters(in: .whitespaces).isEmpty)
                .opacity(newPattern.trimmingCharacters(in: .whitespaces).isEmpty ? 0.4 : 1)
            }
        }
    }

    private var resolvedContextID: BrowserContext.ID? {
        newContextID ?? store.contexts.first?.id
    }

    private var selectedContextName: String {
        store.contexts.first { $0.id == resolvedContextID }?.name ?? "Space"
    }

    private func ruleRow(_ rule: RoutingRule) -> some View {
        HStack(spacing: 10) {
            Icon(name: "arrow.triangle.branch", size: 13)
                .foregroundStyle(p.mutedForeground.color)
            Text(rule.pattern)
                .font(Typography.ui(Typography.base, weight: .medium))
                .foregroundStyle(p.foreground.color)
            Icon(name: "arrow.right", size: 11)
                .foregroundStyle(p.mutedForeground.color)
            Text(store.contexts.first { $0.id == rule.contextID }?.name ?? "—")
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.mutedForeground.color)
            Spacer(minLength: 8)
            Toggle("", isOn: Binding(
                get: { rule.enabled },
                set: { routes.setEnabled($0, for: rule) }))
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.mini)
                .tint(p.primary.color)
            Button { routes.remove(rule) } label: {
                Icon(name: "trash", size: 13)
                    .foregroundStyle(p.mutedForeground.color)
            }
            .buttonStyle(.plain)
            .help("Remove rule")
        }
        .padding(.vertical, 8)
    }

    private func addRule() {
        guard let contextID = resolvedContextID else { return }
        if routes.add(pattern: newPattern, contextID: contextID) {
            newPattern = ""
        }
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
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                            .fill(active ? p.background.color : .clear)
                    )
                    .overlay(
                        RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
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

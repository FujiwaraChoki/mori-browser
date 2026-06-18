import SwiftUI

// MARK: - Store coordination

extension BrowserStore {
    /// Open the Boost editor for the active page's host.
    func presentBoostEditor() {
        guard let tab = selectedTab,
              let host = BoostStore.normalizedHost(forURL: tab.urlString) else {
            ToastCenter.shared.show("Boosts only work on web pages",
                                    icon: "wand.and.stars", style: .warning)
            return
        }
        boostEditorHost = host
        withAnimation(Motion.reveal) { boostEditorVisible = true }
    }

    func dismissBoostEditor() {
        withAnimation(Motion.reveal) { boostEditorVisible = false }
    }

    /// Arm the click-to-zap element picker on the active page.
    func startZapMode() {
        guard let tab = selectedTab,
              BoostStore.normalizedHost(forURL: tab.urlString) != nil else {
            ToastCenter.shared.show("Zapping only works on web pages",
                                    icon: "wand.and.stars", style: .warning)
            return
        }
        guard !zapModeActive else { return }
        if boostEditorVisible { dismissBoostEditor() }
        zapModeActive = true
        tab.injectZapPicker()
        ToastCenter.shared.show("Click elements to remove · Esc to finish",
                                icon: "wand.and.stars", style: .info, duration: 3.5)
    }

    /// Finish zapping: collect the clicked selectors, persist them for the host,
    /// and re-apply so they survive reloads.
    func finishZapMode() {
        guard zapModeActive else { return }
        zapModeActive = false
        guard let tab = selectedTab,
              let host = BoostStore.normalizedHost(forURL: tab.urlString) else { return }
        Task { @MainActor in
            let selectors = await tab.collectZaps()
            guard !selectors.isEmpty else { return }
            BoostStore.shared.addZaps(selectors, host: host)
            tab.applyBoosts()
            ToastCenter.shared.show("Hid \(selectors.count) element\(selectors.count == 1 ? "" : "s")",
                                    icon: "wand.and.stars", style: .success)
        }
    }
}

// MARK: - Editor overlay

/// Centered overlay for editing a site's Boost: enable toggle, zapped-element
/// management, and custom CSS / JS.
struct BoostEditorOverlay: View {
    @ObservedObject var store: BrowserStore
    @Environment(\.palette) private var p

    var body: some View {
        ZStack {
            if store.boostEditorVisible {
                Color.black.opacity(0.28)
                    .ignoresSafeArea()
                    .onTapGesture { store.dismissBoostEditor() }
                    .transition(.opacity)

                BoostEditorCard(store: store, host: store.boostEditorHost)
                    .id(store.boostEditorHost)
                    .transition(.scale(scale: 0.97).combined(with: .opacity))
            }
        }
        .animation(Motion.reveal, value: store.boostEditorVisible)
    }
}

private struct BoostEditorCard: View {
    @ObservedObject var store: BrowserStore
    let host: String
    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme
    @ObservedObject private var boosts = BoostStore.shared

    @State private var css: String = ""
    @State private var js: String = ""
    @State private var enabled: Bool = true

    private var liveBoost: SiteBoost? { boosts.boost(forHost: host) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            header
            Hairline().opacity(0.5)
            ScrollView {
                VStack(alignment: .leading, spacing: 16) {
                    zapSection
                    editorSection(title: "Custom CSS", text: $css,
                                  placeholder: ".ad { display: none; }")
                    editorSection(title: "Custom JavaScript", text: $js,
                                  placeholder: "document.title = 'Boosted';")
                }
                .padding(Spacing.xl)
            }
            Hairline().opacity(0.5)
            footer
        }
        .frame(width: 460, height: 540)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(Stroke.border), lineWidth: 1)
        )
        .elevation(.overlay, scheme)
        .onAppear(perform: loadState)
    }

    private var header: some View {
        HStack(spacing: 10) {
            Icon(name: "wand.and.stars", size: 16)
                .foregroundStyle(p.accent.color)
            VStack(alignment: .leading, spacing: 1) {
                Text("Boost")
                    .font(Typography.ui(Typography.title, weight: .semibold))
                    .foregroundStyle(p.popoverForeground.color)
                Text(host)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer()
            Toggle("", isOn: $enabled)
                .labelsHidden()
                .toggleStyle(.switch)
                .controlSize(.small)
                .help("Enable this Boost")
        }
        .padding(.horizontal, Spacing.xl)
        .frame(height: 56)
    }

    private var zapSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Zapped Elements")
                .font(Typography.ui(11, weight: .semibold))
                .foregroundStyle(.secondary)
            HStack(spacing: 8) {
                Button {
                    store.startZapMode()
                } label: {
                    Label("Zap an element", systemImage: "scope")
                        .font(Typography.ui(Typography.base, weight: .medium))
                }
                .buttonStyle(.borderedProminent)
                .controlSize(.small)

                let count = liveBoost?.zappedSelectors.count ?? 0
                Text("\(count) hidden")
                    .font(Typography.ui(11))
                    .foregroundStyle(.secondary)
                Spacer()
                if count > 0 {
                    Button("Clear") { boosts.clearZaps(host: host) }
                        .buttonStyle(.plain)
                        .font(Typography.ui(11, weight: .medium))
                        .foregroundStyle(p.destructive.color)
                }
            }
        }
    }

    private func editorSection(title: String, text: Binding<String>, placeholder: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Text(title)
                .font(Typography.ui(11, weight: .semibold))
                .foregroundStyle(.secondary)
            ZStack(alignment: .topLeading) {
                if text.wrappedValue.isEmpty {
                    Text(placeholder)
                        .font(Typography.mono(12))
                        .foregroundStyle(p.mutedForeground.color.opacity(0.5))
                        .padding(.horizontal, 9)
                        .padding(.vertical, 9)
                        .allowsHitTesting(false)
                }
                TextEditor(text: text)
                    .font(Typography.mono(12))
                    .scrollContentBackground(.hidden)
                    .padding(.horizontal, 5)
                    .padding(.vertical, 4)
            }
            .frame(height: 140)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(p.muted.color.opacity(0.5))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(p.border.color.opacity(Stroke.border), lineWidth: 1)
            )
        }
    }

    private var footer: some View {
        HStack {
            Button("Remove Boost", role: .destructive) {
                boosts.remove(host: host)
                store.selectedTab?.reload()
                store.dismissBoostEditor()
            }
            .buttonStyle(.plain)
            .font(Typography.ui(Typography.base, weight: .medium))
            .foregroundStyle(p.destructive.color)
            .opacity(liveBoost == nil ? 0.4 : 1)
            .disabled(liveBoost == nil)

            Spacer()
            Button("Cancel") { store.dismissBoostEditor() }
                .keyboardShortcut(.cancelAction)
            Button("Save", action: save)
                .keyboardShortcut(.defaultAction)
                .buttonStyle(.borderedProminent)
        }
        .padding(.horizontal, Spacing.xl)
        .frame(height: 56)
    }

    private func loadState() {
        let boost = boosts.editableBoost(forHost: host)
        css = boost.css
        js = boost.js
        enabled = boost.enabled
    }

    private func save() {
        var boost = boosts.editableBoost(forHost: host)
        boost.css = css
        boost.js = js
        boost.enabled = enabled
        if boost.isEmpty {
            boosts.remove(host: host)
        } else {
            boosts.upsert(boost)
        }
        store.selectedTab?.applyBoosts()
        store.selectedTab?.reload()
        store.dismissBoostEditor()
    }
}

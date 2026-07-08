import SwiftUI

// Shared conversation UI used by BOTH the 360pt `AIPanel` side panel and the
// full-page `AgentThreadView`, so the two stay visually identical and DRY.
// The message row (`AIBubble`), tool-call chip/popover, and data models live in
// AIPanel.swift (same module); these are the larger reusable surfaces.

/// The scrolling transcript: a column of `AIBubble`s that auto-scrolls to the
/// bottom as messages and streaming text arrive. Pass `maxContentWidth` to get a
/// centered reading column (the full page); leave it `nil` for the full-width
/// panel.
struct AITranscript: View {
    let messages: [AIMessage]
    var maxContentWidth: CGFloat? = nil
    var horizontalPadding: CGFloat = 16

    var body: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(messages) { msg in
                        AIBubble(message: msg)
                            .id(msg.id)
                    }
                }
                .frame(maxWidth: maxContentWidth ?? .infinity, alignment: .leading)
                .frame(maxWidth: .infinity, alignment: .center)
                .padding(.horizontal, horizontalPadding)
                .padding(.vertical, 16)
            }
            .onChange(of: messages.count) { _, _ in scrollToBottom(proxy) }
            .onChange(of: messages.last?.text ?? "") { _, _ in scrollToBottom(proxy) }
        }
    }

    private func scrollToBottom(_ proxy: ScrollViewProxy) {
        guard let id = messages.last?.id else { return }
        withAnimation(Motion.state) { proxy.scrollTo(id, anchor: .bottom) }
    }
}

/// The message composer: a vertically-growing text field plus a send button on a
/// liquid-glass pill. Owns its own focus and clears the field on submit.
struct AIComposer: View {
    @Binding var text: String
    var placeholder: String = "Ask anything…"
    let isWorking: Bool
    let onStop: () -> Void
    let onSend: (String) -> Void

    @Environment(\.palette) private var p
    @FocusState private var focused: Bool

    private var sendDisabled: Bool {
        text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || isWorking
    }

    private func submit() {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty, !isWorking else { return }
        text = ""
        onSend(trimmed)
    }

    var body: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField(placeholder, text: $text, axis: .vertical)
                .textFieldStyle(.plain)
                .font(Typography.ui(Typography.base))
                .tint(p.accent.color)
                .lineLimit(1...6)
                .padding(.vertical, 6)
                .focused($focused)
                .onSubmit(submit)

            if isWorking {
                Button(action: onStop) {
                    Icon(name: "stop.fill", size: 13, weight: .bold)
                        .foregroundStyle(p.statusWarningFg.color)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .help("Stop")
            } else {
                Button(action: submit) {
                    Icon(name: "paper.plane", size: 15, weight: .bold)
                        .foregroundStyle(sendDisabled ? p.mutedForeground.color.opacity(0.5) : p.accent.color)
                        .frame(width: 28, height: 28)
                        .contentShape(Rectangle())
                }
                .buttonStyle(.plain)
                .disabled(sendDisabled)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background {
            Color.clear.liquidGlass(cornerRadius: Radius.popover, interactive: true)
        }
        .contentShape(RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
        .onTapGesture { focused = true }
        .padding(12)
    }
}

/// The model + reasoning-effort dropdowns, borderless to sit quietly under a
/// transcript. Disabled while a turn is in flight.
struct AIModelSelectors: View {
    @ObservedObject var assistant: CodexBrowserAssistant
    @Environment(\.palette) private var p

    var body: some View {
        HStack(spacing: 8) {
            Menu {
                if assistant.modelOptions.isEmpty {
                    Button(modelSelectorTitle) {}
                } else {
                    ForEach(assistant.modelOptions) { model in
                        Button(model.displayName) { assistant.selectedModelID = model.id }
                    }
                }
            } label: {
                selectorLabel(modelSelectorTitle)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .disabled(assistant.isWorking || assistant.modelOptions.isEmpty)
            .opacity(assistant.isWorking || assistant.modelOptions.isEmpty ? 0.55 : 1)

            Menu {
                if assistant.reasoningEffortOptions.isEmpty {
                    Button("Default Effort") {}
                } else {
                    ForEach(assistant.reasoningEffortOptions) { effort in
                        Button(effort.displayName) { assistant.selectedReasoningEffort = effort.id }
                    }
                }
            } label: {
                selectorLabel(effortSelectorTitle)
            }
            .menuStyle(.borderlessButton)
            .menuIndicator(.hidden)
            .fixedSize()
            .disabled(assistant.isWorking || assistant.reasoningEffortOptions.isEmpty)
            .opacity(assistant.isWorking || assistant.reasoningEffortOptions.isEmpty ? 0.55 : 1)
        }
    }

    private var modelSelectorTitle: String {
        assistant.modelOptions.first(where: { $0.id == assistant.selectedModelID })?.displayName
            ?? (assistant.isLoadingModels ? "Loading Models" : "Default Model")
    }

    private var effortSelectorTitle: String {
        assistant.reasoningEffortOptions.first(where: { $0.id == assistant.selectedReasoningEffort })?.displayName
            ?? "Default Effort"
    }

    private func selectorLabel(_ title: String) -> some View {
        HStack(spacing: 4) {
            Text(title)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
                .lineLimit(1)
            Image(systemName: "chevron.down")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(p.foreground.color)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.button, style: .continuous))
    }
}

/// Non-modal browser-tool approval, rendered inline in the thread. Visual
/// language mirrors `PermissionPromptCard` (the site-permission card) so consent
/// surfaces feel consistent. Allow/Deny resolve the assistant's parked turn.
struct AIInlineApprovalCard: View {
    let approval: CodexPendingApproval
    let onAllow: () -> Void
    let onDeny: () -> Void

    @ObservedObject private var settings = BrowserSettings.shared
    @Environment(\.palette) private var p
    @Environment(\.colorScheme) private var scheme

    private var request: BrowserToolApprovalRequest { approval.request }
    private var tint: Color {
        request.isDestructive ? p.statusWarningFg.color : p.statusInfoFg.color
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack(alignment: .top, spacing: 10) {
                Icon(name: "sparkles", size: 15)
                    .foregroundStyle(tint)
                    .frame(width: 22, height: 22)
                    .background(Circle().fill(tint.opacity(scheme == .dark ? 0.18 : 0.12)))

                VStack(alignment: .leading, spacing: 6) {
                    Text(request.title)
                        .font(Typography.ui(Typography.base, weight: .semibold))
                        .foregroundStyle(p.foreground.color)
                        .fixedSize(horizontal: false, vertical: true)
                    Text(request.message)
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }

            // Only offered for read-only tools — write/navigation actions always
            // ask, regardless of this setting.
            if !request.isDestructive {
                Toggle(isOn: $settings.agentAutoApproveSafeReads) {
                    Text("Auto-approve safe reads from now on")
                        .font(Typography.ui(Typography.label))
                        .foregroundStyle(p.mutedForeground.color)
                }
                .toggleStyle(.switch)
                .controlSize(.mini)
            }

            HStack(spacing: 8) {
                AIPromptButton("Deny", action: onDeny)
                AIPromptButton(request.confirmButtonTitle, primary: true, action: onAllow)
            }
        }
        .padding(16)
        .background(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .fill(p.popover.color)
        )
        .overlay(
            RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                .strokeBorder(p.border.color.opacity(Stroke.border), lineWidth: 1)
        )
        .elevation(.popover, scheme)
    }
}

/// Pill button used in the inline approval card. Mirrors ToastOverlay's private
/// `PromptButton` so the two consent surfaces look identical.
struct AIPromptButton: View {
    let title: String
    var primary = false
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    init(_ title: String, primary: Bool = false, action: @escaping () -> Void) {
        self.title = title
        self.primary = primary
        self.action = action
    }

    var body: some View {
        Button(action: action) {
            Text(title)
                .font(Typography.ui(Typography.label, weight: .medium))
                .foregroundStyle(primary ? p.primaryForeground.color : p.foreground.color)
                .lineLimit(1)
                .frame(maxWidth: .infinity)
                .frame(height: 28)
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(background)
                )
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .strokeBorder(p.border.color.opacity(Stroke.border), lineWidth: primary ? 0 : 1)
                )
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
    }

    private var background: Color {
        if primary {
            return hovering ? p.primary.color.opacity(0.9) : p.primary.color
        }
        return hovering ? p.accent.color : p.muted.color
    }
}

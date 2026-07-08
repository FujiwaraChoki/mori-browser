import SwiftUI
import AppKit
import AVFoundation

/// The AI assistant side panel. Talks to the local Codex app server and exposes
/// Mori browser tools for page reading and user-like actions.
struct AIPanel: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject private var assistant: CodexBrowserAssistant
    @Environment(\.palette) private var p
    @State private var draft: String = ""
    @State private var historyOpen: Bool = false

    @MainActor
    init(store: BrowserStore) {
        self.store = store
        _assistant = ObservedObject(wrappedValue: store.sidePanelAssistant)
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.6)
            if let error = assistant.modelLoadError {
                errorBanner(error)
            }
            transcript
            if let approval = assistant.pendingApproval {
                AIInlineApprovalCard(approval: approval,
                                     onAllow: { assistant.resolveApproval(true) },
                                     onDeny: { assistant.resolveApproval(false) })
                    .padding(.horizontal, 12)
                    .padding(.top, 8)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
            AIModelSelectors(assistant: assistant)
                .padding(.horizontal, 12)
                .padding(.top, 8)
                .frame(maxWidth: .infinity, alignment: .leading)
            AIComposer(text: $draft,
                       isWorking: assistant.isWorking,
                       onStop: { assistant.stop() }) { assistant.send($0) }
        }
        .frame(width: 360)
        .animation(Motion.state, value: assistant.pendingApproval?.id)
        .task { await assistant.loadModelCatalogIfNeeded() }
        // No own background: the unified chrome surface (set on the root) shows
        // through, so the panel follows the selected theme like the sidebar.
    }

    private var header: some View {
        HStack(spacing: 8) {
            Icon(name: "sparkles", size: 16)
                .foregroundStyle(p.mutedForeground.color)

            Text("Assistant")
                .font(Typography.ui(Typography.title, weight: .semibold))
                .foregroundStyle(p.foreground.color)

            if assistant.totalTokensUsed > 0 {
                Text(assistant.tokenUsageLabel)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
            }

            Spacer()

            if assistant.isWorking {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.65)
            }
            IconButton(systemName: "magnifier-history", size: 28) { historyOpen.toggle() }
                .help("Conversation history")
                .popover(isPresented: $historyOpen, arrowEdge: .top) {
                    AIHistoryPopover(assistant: assistant) {
                        historyOpen = false
                    }
                }
            IconButton(systemName: "xmark", size: 28) { store.toggleAIPanel() }
                .help("Close assistant")
        }
        .padding(.horizontal, 14)
        .frame(height: 48)
    }

    /// Inline banner shown when the local Codex server can't be reached, with a
    /// one-tap retry so the user isn't stuck staring at an empty panel.
    private func errorBanner(_ message: String) -> some View {
        HStack(alignment: .top, spacing: 8) {
            Icon(name: "exclamationmark.triangle", size: 13, weight: .medium)
                .foregroundStyle(p.statusWarningFg.color)
            Text(message)
                .font(Typography.ui(Typography.small))
                .foregroundStyle(p.foreground.color.opacity(0.9))
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 6)
            Button("Retry") {
                Task { await assistant.loadModelCatalogIfNeeded() }
            }
            .font(Typography.ui(Typography.small, weight: .medium))
            .buttonStyle(.plain)
            .foregroundStyle(p.accent.color)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(p.statusWarningFg.color.opacity(0.10))
        .overlay(alignment: .bottom) { Hairline().opacity(0.6) }
    }

    @ViewBuilder
    private var transcript: some View {
        if assistant.messages.isEmpty {
            emptyState
        } else {
            AITranscript(messages: assistant.messages)
        }
    }

    /// Welcome state shown before the first message: a short prompt plus the
    /// one-tap page-grounded suggestions, centered in the conversation area so
    /// they read as an invitation rather than clutter wedged above the input.
    private var emptyState: some View {
        VStack(spacing: 16) {
            Icon(name: "sparkles", size: 30)
                .foregroundStyle(p.accent.color)
            VStack(spacing: 4) {
                Text("Ask about this page")
                    .font(Typography.ui(Typography.title, weight: .semibold))
                    .foregroundStyle(p.foreground.color)
                Text("Summarize it, pull the key points, or tidy your tabs.")
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .multilineTextAlignment(.center)
            }
            FlowLayout(spacing: 7, lineSpacing: 7) {
                quickChip("Summarize", icon: "doc.text") {
                    runQuick("Summarize the current page concisely in a few sentences.")
                }
                quickChip("Key points", icon: "list.bullet") {
                    runQuick("List the key points of the current page as concise bullet points.")
                }
                quickChip("Tidy tabs", icon: "rectangle.3.group") {
                    runQuick("Look at all my open tabs and organize them into sensible, "
                             + "short-named sidebar folders using the mori_organize_tabs tool. "
                             + "Group by topic or site.")
                }
            }
            .frame(maxWidth: .infinity)
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .opacity(assistant.isWorking ? 0.5 : 1)
        .disabled(assistant.isWorking)
    }

    private func quickChip(_ title: String, icon: String, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            HStack(spacing: 5) {
                Icon(name: icon, size: 11, weight: .medium)
                Text(title).font(Typography.ui(Typography.small, weight: .medium))
            }
            .foregroundStyle(p.foreground.color.opacity(0.85))
            .padding(.horizontal, 9)
            .frame(height: 24)
            .background(.regularMaterial,
                        in: Capsule())
            .overlay(Capsule().strokeBorder(p.border.color.opacity(0.4), lineWidth: 1))
        }
        .buttonStyle(.plain)
    }

    private func runQuick(_ prompt: String) {
        guard !assistant.isWorking else { return }
        store.openAIPanel()
        assistant.send(prompt)
    }

}

/// A simple wrapping row layout that center-aligns each line — used for the
/// assistant's suggestion chips so they reflow instead of clipping in the
/// fixed-width panel.
private struct FlowLayout: Layout {
    var spacing: CGFloat = 7
    var lineSpacing: CGFloat = 7

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let maxWidth = proposal.width ?? .infinity
        let rows = rows(maxWidth: maxWidth, subviews: subviews)
        let width = rows.map(\.width).max() ?? 0
        let height = rows.map(\.height).reduce(0, +)
            + lineSpacing * CGFloat(max(0, rows.count - 1))
        return CGSize(width: min(width, maxWidth), height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        var y = bounds.minY
        for row in rows(maxWidth: bounds.width, subviews: subviews) {
            var x = bounds.minX + (bounds.width - row.width) / 2
            for index in row.indices {
                let size = subviews[index].sizeThatFits(.unspecified)
                subviews[index].place(at: CGPoint(x: x, y: y),
                                      anchor: .topLeading,
                                      proposal: ProposedViewSize(size))
                x += size.width + spacing
            }
            y += row.height + lineSpacing
        }
    }

    private struct Row { var indices: [Int] = []; var width: CGFloat = 0; var height: CGFloat = 0 }

    private func rows(maxWidth: CGFloat, subviews: Subviews) -> [Row] {
        var rows: [Row] = []
        var current = Row()
        for index in subviews.indices {
            let size = subviews[index].sizeThatFits(.unspecified)
            let projected = current.indices.isEmpty ? size.width : current.width + spacing + size.width
            if projected > maxWidth, !current.indices.isEmpty {
                rows.append(current)
                current = Row(indices: [index], width: size.width, height: size.height)
            } else {
                if !current.indices.isEmpty { current.width += spacing }
                current.indices.append(index)
                current.width += size.width
                current.height = max(current.height, size.height)
            }
        }
        if !current.indices.isEmpty { rows.append(current) }
        return rows
    }
}

private struct AIHistoryPopover: View {
    @ObservedObject var assistant: CodexBrowserAssistant
    var onSelect: () -> Void
    @Environment(\.palette) private var p
    @State private var searchText = ""
    @FocusState private var searchFocused: Bool

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Conversation History")
                .font(Typography.ui(13, weight: .semibold))
                .foregroundStyle(p.popoverForeground.color)

            HStack(spacing: 7) {
                Icon(name: "magnifyingglass", size: 13)
                    .foregroundStyle(p.mutedForeground.color)
                TextField("Search conversations", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(Typography.base))
                    .focused($searchFocused)
                    .onAppear { searchFocused = true }
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .fill(p.muted.color.opacity(0.55))
            )
            .overlay(
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.55), lineWidth: 1)
            )

            ZStack {
                if assistant.conversationHistory.isEmpty,
                   !assistant.isLoadingHistory {
                    let emptyLabel = assistant.historyError
                        ?? (searchText.isEmpty ? "No conversations yet" : "No matching conversations")
                    Text(emptyLabel)
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(p.mutedForeground.color)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                } else {
                    ScrollView {
                        LazyVStack(alignment: .leading, spacing: 4) {
                            ForEach(assistant.conversationHistory) { conversation in
                                Button {
                                    Task { @MainActor in
                                        await assistant.openConversation(conversation)
                                        onSelect()
                                    }
                                } label: {
                                    AIHistoryRow(conversation: conversation)
                                }
                                .buttonStyle(.plain)
                                .disabled(assistant.isWorking)
                                .opacity(assistant.isWorking ? 0.5 : 1)
                            }
                        }
                    }
                }

                if assistant.isLoadingHistory {
                    ProgressView()
                        .controlSize(.small)
                        .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .frame(height: 270)
        }
        .padding(12)
        .frame(width: 320)
        .background(p.popover.color)
        .task(id: searchText) {
            if !searchText.isEmpty {
                try? await Task.sleep(nanoseconds: 250_000_000)
            }
            guard !Task.isCancelled else { return }
            await assistant.loadConversationHistory(searchTerm: searchText)
        }
    }
}

private struct AIHistoryRow: View {
    let conversation: CodexConversationSummary
    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(conversation.title)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.popoverForeground.color)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text(conversation.updatedAt, format: .relative(presentation: .named, unitsStyle: .abbreviated))
                    .font(Typography.ui(Typography.caption))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(1)
            }
            if !conversation.preview.isEmpty && conversation.preview != conversation.title {
                Text(conversation.preview)
                    .font(Typography.ui(Typography.small))
                    .foregroundStyle(p.mutedForeground.color)
                    .lineLimit(2)
            }
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(RoundedRectangle(cornerRadius: Radius.md, style: .continuous))
        .background(
            RoundedRectangle(cornerRadius: Radius.md, style: .continuous)
                .fill(hovering ? p.foreground.color.opacity(0.07) : .clear)
        )
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
    }
}

struct AIMessage: Identifiable {
    enum Role { case user, assistant, tool }
    let id = UUID()
    let role: Role
    var text: String
    var toolCall: AIToolCallInfo?

    init(role: Role, text: String, toolCall: AIToolCallInfo? = nil) {
        self.role = role
        self.text = text
        self.toolCall = toolCall
    }
}

struct AIToolCallInfo: Equatable {
    var title: String
    var name: String
    var arguments: String
    var reason: String?
    var result: String?
    var success: Bool?
}

struct AIBubble: View {
    let message: AIMessage
    @ObservedObject private var speech = SpeechCenter.shared
    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        HStack {
            if message.role == .user { Spacer(minLength: 32) }
            VStack(alignment: message.role == .user ? .trailing : .leading, spacing: 3) {
                bubbleContent
                if showsActions {
                    actionRow
                        .opacity(hovering || isSpeaking ? 1 : 0)
                        .animation(Motion.state, value: hovering)
                        .animation(Motion.state, value: isSpeaking)
                }
            }
            if message.role != .user { Spacer(minLength: 32) }
        }
        .onHover { hovering = $0 }
    }

    /// Copy (and, for assistant replies, speak-aloud) controls beneath a message.
    @ViewBuilder
    private var actionRow: some View {
        HStack(spacing: 2) {
            MessageActionButton(icon: "doc.on.doc", help: "Copy") {
                copyText()
            }
            if message.role == .assistant {
                MessageActionButton(icon: isSpeaking ? "stop.fill" : "speaker.wave.2.fill",
                                    help: isSpeaking ? "Stop" : "Speak aloud") {
                    speech.toggle(message.text, id: message.id)
                }
            }
        }
        .padding(.horizontal, message.role == .user ? 4 : 0)
    }

    private var showsActions: Bool {
        message.toolCall == nil && !isLoading && !message.text.isEmpty
    }

    private var isSpeaking: Bool {
        message.role == .assistant && speech.isSpeaking(message.id)
    }

    private func copyText() {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(message.text, forType: .string)
        ToastCenter.shared.show("Copied to clipboard", icon: "doc.on.doc", style: .success)
    }

    @ViewBuilder
    private var bubbleContent: some View {
        if let toolCall = message.toolCall {
            AIToolCallButton(toolCall: toolCall)
        } else if isLoading {
            AILoadingDot()
                .padding(.horizontal, 4)
                .padding(.vertical, 6)
        } else if message.role == .assistant {
            Text(message.text)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
                .textSelection(.enabled)
                .padding(.horizontal, 2)
                .padding(.vertical, 4)
        } else {
            Text(message.text)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(p.foreground.color)
                .textSelection(.enabled)
                .padding(.horizontal, 12)
                .padding(.vertical, 9)
                .background(.regularMaterial,
                            in: RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
                .overlay(
                    RoundedRectangle(cornerRadius: Radius.popover, style: .continuous)
                        .strokeBorder(Color.primary.opacity(0.12), lineWidth: 1)
                )
        }
    }

    private var isLoading: Bool {
        message.role == .assistant && message.text.isEmpty
    }
}

private struct AIToolCallButton: View {
    let toolCall: AIToolCallInfo
    @Environment(\.palette) private var p
    @State private var showingDetails = false

    var body: some View {
        Button {
            showingDetails.toggle()
        } label: {
            HStack(spacing: 7) {
                Circle()
                    .fill(statusColor)
                    .frame(width: 6, height: 6)
                Text(toolCall.title)
                    .font(Typography.ui(12, weight: .medium))
                    .lineLimit(1)
                Icon(name: "chevron.down", size: 10)
                    .opacity(0.6)
            }
            .foregroundStyle(p.foreground.color)
            .padding(.horizontal, 10)
            .padding(.vertical, 7)
            .background {
                Color.clear.liquidGlass(cornerRadius: Radius.button, interactive: true)
            }
            .overlay(
                RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                    .strokeBorder(Color.primary.opacity(0.12), lineWidth: 1)
            )
        }
        .buttonStyle(.plain)
        .popover(isPresented: $showingDetails, arrowEdge: .bottom) {
            AIToolCallPopover(toolCall: toolCall)
        }
    }

    private var statusColor: Color {
        switch toolCall.success {
        case .some(true): return p.statusSuccessFg.color.opacity(0.85)
        case .some(false): return p.destructive.color.opacity(0.85)
        case .none: return p.mutedForeground.color.opacity(0.8)
        }
    }
}

private struct AIToolCallPopover: View {
    let toolCall: AIToolCallInfo
    @Environment(\.palette) private var p

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(toolCall.name)
                .font(Typography.ui(13, weight: .semibold))
                .foregroundStyle(p.popoverForeground.color)
            if let reason = toolCall.reason, !reason.isEmpty {
                detailBlock(title: "Reason", text: reason)
            }
            detailBlock(title: "Arguments", text: toolCall.arguments.isEmpty ? "{}" : toolCall.arguments)
            if let result = toolCall.result, !result.isEmpty {
                detailBlock(title: toolCall.success == false ? "Error" : "Result", text: result)
            }
        }
        .padding(12)
        .frame(width: 280, alignment: .leading)
    }

    private func detailBlock(title: String, text: String) -> some View {
        VStack(alignment: .leading, spacing: 3) {
            Text(title)
                .font(Typography.ui(Typography.caption, weight: .semibold))
                .foregroundStyle(p.mutedForeground.color)
            Text(text)
                .font(Typography.ui(Typography.small))
                .foregroundStyle(p.popoverForeground.color)
                .lineLimit(8)
                .textSelection(.enabled)
        }
    }
}

/// Compact ghost button used for the per-message copy / speak-aloud affordances.
/// Smaller than `IconButton` so it tucks neatly under a chat bubble.
private struct MessageActionButton: View {
    let icon: String
    let help: String
    let action: () -> Void

    @Environment(\.palette) private var p
    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Icon(name: icon, size: 12)
                .frame(width: 22, height: 22)
                .foregroundStyle(p.foreground.color.opacity(hovering ? 0.9 : 0.55))
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(p.foreground.color.opacity(hovering ? 0.08 : 0))
                )
                .contentShape(Rectangle())
        }
        .buttonStyle(.plain)
        .onHover { hovering = $0 }
        .animation(Motion.state, value: hovering)
        .help(help)
    }
}

/// App-wide text-to-speech for AI replies. One synthesizer, one active
/// utterance at a time; views observe `speakingMessageID` to flip their
/// speak/stop control.
@MainActor
final class SpeechCenter: ObservableObject {
    static let shared = SpeechCenter()

    @Published private(set) var speakingMessageID: UUID?

    private let synthesizer = AVSpeechSynthesizer()
    private var delegateProxy: Delegate?

    private init() {
        let proxy = Delegate { [weak self] in
            Task { @MainActor in self?.finished() }
        }
        delegateProxy = proxy
        synthesizer.delegate = proxy
    }

    func isSpeaking(_ id: UUID) -> Bool { speakingMessageID == id }

    /// Speak `text` for `id`, or stop if that message is already being read.
    func toggle(_ text: String, id: UUID) {
        if speakingMessageID == id {
            stop()
            return
        }
        stop()
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return }
        let utterance = AVSpeechUtterance(string: trimmed)
        utterance.rate = AVSpeechUtteranceDefaultSpeechRate
        speakingMessageID = id
        synthesizer.speak(utterance)
    }

    func stop() {
        if synthesizer.isSpeaking {
            synthesizer.stopSpeaking(at: .immediate)
        }
        speakingMessageID = nil
    }

    fileprivate func finished() { speakingMessageID = nil }

    private final class Delegate: NSObject, AVSpeechSynthesizerDelegate {
        private let onFinish: @Sendable () -> Void

        init(onFinish: @escaping @Sendable () -> Void) {
            self.onFinish = onFinish
        }

        func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
            onFinish()
        }
        func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
            onFinish()
        }
    }
}

private struct AILoadingDot: View {
    @Environment(\.palette) private var p
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isPulsing = false

    var body: some View {
        Circle()
            .fill(p.foreground.color.opacity(isPulsing ? 0.85 : 0.28))
            .frame(width: 7, height: 7)
            .scaleEffect(isPulsing ? 1.35 : 0.72)
            .frame(width: 18, height: 18)
            .animation(reduceMotion ? nil : Motion.pulse, value: isPulsing)
            .onAppear {
                guard !reduceMotion else { return }
                isPulsing = true
            }
            .accessibilityLabel("Assistant is thinking")
    }
}

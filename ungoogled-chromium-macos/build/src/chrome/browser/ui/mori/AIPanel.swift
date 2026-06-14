import SwiftUI
import AppKit
import AVFoundation

/// The AI assistant side panel. Talks to the local Codex app server and exposes
/// Mori browser tools for page reading and user-like actions.
struct AIPanel: View {
    @ObservedObject var store: BrowserStore
    @StateObject private var assistant: CodexBrowserAssistant
    @Environment(\.palette) private var p
    @FocusState private var inputFocused: Bool
    @State private var draft: String = ""
    @State private var historyOpen: Bool = false

    init(store: BrowserStore) {
        self.store = store
        _assistant = StateObject(wrappedValue: CodexBrowserAssistant(store: store))
    }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider().opacity(0.6)
            transcript
            modelSelectors
            composer
        }
        .frame(width: 360)
        .task { await assistant.loadModelCatalogIfNeeded() }
        // No own background: the unified chrome surface (set on the root) shows
        // through, so the panel follows the selected theme like the sidebar.
    }

    private var header: some View {
        HStack(spacing: 8) {
            Icon(name: "sparkles", size: 16)
                .foregroundStyle(.secondary)

            Text("Assistant")
                .font(Typography.ui(15, weight: .semibold))
                .foregroundStyle(.primary)

            Spacer()

            if assistant.isWorking {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.65)
            }
            Button(action: { historyOpen.toggle() }) {
                Icon(name: "magnifier-history", size: 17)
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Conversation history")
            .popover(isPresented: $historyOpen, arrowEdge: .top) {
                AIHistoryPopover(assistant: assistant) {
                    historyOpen = false
                }
            }
            Button(action: { store.toggleAIPanel() }) {
                Icon(name: "xmark", size: 16)
                    .foregroundStyle(.secondary)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Close assistant")
        }
        .padding(.horizontal, 14)
        .frame(height: 48)
    }

    private var transcript: some View {
        ScrollViewReader { proxy in
            ScrollView {
                VStack(alignment: .leading, spacing: 14) {
                    ForEach(assistant.messages) { msg in
                        AIBubble(message: msg)
                            .id(msg.id)
                    }
                }
                .padding(16)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
            .onChange(of: assistant.messages.count) { _, _ in
                scrollToBottom(proxy)
            }
            .onChange(of: assistant.messages.last?.text ?? "") { _, _ in
                scrollToBottom(proxy)
            }
        }
    }

    private var modelSelectors: some View {
        HStack(spacing: 8) {
            Menu {
                if assistant.modelOptions.isEmpty {
                    Button(modelSelectorTitle) {}
                } else {
                    ForEach(assistant.modelOptions) { model in
                        Button(model.displayName) {
                            assistant.selectedModelID = model.id
                        }
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
                        Button(effort.displayName) {
                            assistant.selectedReasoningEffort = effort.id
                        }
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
        .padding(.horizontal, 12)
        .padding(.top, 8)
        .frame(maxWidth: .infinity, alignment: .leading)
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
                .font(Typography.ui(16))
                .foregroundStyle(.primary)
                .lineLimit(1)
            Image(systemName: "chevron.down")
                .font(.system(size: 11, weight: .semibold))
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 10)
        .padding(.vertical, 6)
        .background(.regularMaterial, in: RoundedRectangle(cornerRadius: Radius.button, style: .continuous))
    }

    private var composer: some View {
        HStack(alignment: .bottom, spacing: 8) {
            TextField("Ask anything...", text: $draft, axis: .vertical)
                .textFieldStyle(.plain)
                .font(Typography.ui(Typography.base))
                .tint(p.accent.color)
                .lineLimit(1...6)
                .padding(.vertical, 6)
                .focused($inputFocused)
                .onSubmit(send)

            Button(action: send) {
                Icon(name: "paper.plane", size: 15, weight: .bold)
                    .foregroundStyle(sendDisabled ? p.mutedForeground.color.opacity(0.5) : p.accent.color)
                    .frame(width: 28, height: 28)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .disabled(sendDisabled)
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 8)
        .background {
            Color.clear.liquidGlass(cornerRadius: Radius.popover, interactive: true)
        }
        .contentShape(RoundedRectangle(cornerRadius: Radius.popover, style: .continuous))
        .onTapGesture { inputFocused = true }
        .padding(12)
    }

    private var sendDisabled: Bool {
        draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || assistant.isWorking
    }

    private func send() {
        let text = draft.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return }
        draft = ""
        assistant.send(text)
    }

    private func scrollToBottom(_ proxy: ScrollViewProxy) {
        guard let id = assistant.messages.last?.id else { return }
        withAnimation(Motion.state) { proxy.scrollTo(id, anchor: .bottom) }
    }
}

private struct AIHistoryPopover: View {
    @ObservedObject var assistant: CodexBrowserAssistant
    var onSelect: () -> Void
    @Environment(\.palette) private var p
    @State private var searchText = ""

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Conversation History")
                .font(Typography.ui(13, weight: .semibold))
                .foregroundStyle(p.popoverForeground.color)

            HStack(spacing: 7) {
                Icon(name: "magnifyingglass", size: 13)
                    .foregroundStyle(.secondary)
                TextField("Search conversations", text: $searchText)
                    .textFieldStyle(.plain)
                    .font(Typography.ui(Typography.base))
            }
            .padding(.horizontal, 9)
            .padding(.vertical, 7)
            .background(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .fill(p.muted.color.opacity(0.55))
            )
            .overlay(
                RoundedRectangle(cornerRadius: 8, style: .continuous)
                    .strokeBorder(p.border.color.opacity(0.55), lineWidth: 1)
            )

            ZStack {
                if assistant.conversationHistory.isEmpty,
                   !assistant.isLoadingHistory {
                    Text(assistant.historyError ?? "No conversations found")
                        .font(Typography.ui(Typography.base))
                        .foregroundStyle(.secondary)
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

    var body: some View {
        VStack(alignment: .leading, spacing: 3) {
            HStack(alignment: .firstTextBaseline, spacing: 8) {
                Text(conversation.title)
                    .font(Typography.ui(Typography.base, weight: .medium))
                    .foregroundStyle(p.popoverForeground.color)
                    .lineLimit(1)
                Spacer(minLength: 8)
                Text(conversation.updatedAt.formatted(date: .abbreviated, time: .shortened))
                    .font(Typography.ui(10))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            if !conversation.preview.isEmpty && conversation.preview != conversation.title {
                Text(conversation.preview)
                    .font(Typography.ui(11))
                    .foregroundStyle(.secondary)
                    .lineLimit(2)
            }
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
        .frame(maxWidth: .infinity, alignment: .leading)
        .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
        .background(
            RoundedRectangle(cornerRadius: 8, style: .continuous)
                .fill(Color.primary.opacity(0.0001))
        )
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
                .foregroundStyle(.primary)
                .textSelection(.enabled)
                .padding(.horizontal, 2)
                .padding(.vertical, 4)
        } else {
            Text(message.text)
                .font(Typography.ui(Typography.base))
                .foregroundStyle(.primary)
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
            .foregroundStyle(.primary)
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
        case .some(true): return .green.opacity(0.85)
        case .some(false): return .red.opacity(0.85)
        case .none: return .secondary.opacity(0.8)
        }
    }
}

private struct AIToolCallPopover: View {
    let toolCall: AIToolCallInfo

    var body: some View {
        VStack(alignment: .leading, spacing: 9) {
            Text(toolCall.name)
                .font(Typography.ui(13, weight: .semibold))
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
                .font(Typography.ui(10, weight: .semibold))
                .foregroundStyle(.secondary)
            Text(text)
                .font(Typography.ui(11))
                .foregroundStyle(.primary)
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

    @State private var hovering = false

    var body: some View {
        Button(action: action) {
            Icon(name: icon, size: 12)
                .frame(width: 22, height: 22)
                .foregroundStyle(Color.primary.opacity(hovering ? 0.9 : 0.55))
                .background(
                    RoundedRectangle(cornerRadius: Radius.button, style: .continuous)
                        .fill(Color.primary.opacity(hovering ? 0.08 : 0))
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
        let proxy = Delegate(owner: self)
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
        weak var owner: SpeechCenter?
        init(owner: SpeechCenter) { self.owner = owner }

        func speechSynthesizer(_ s: AVSpeechSynthesizer, didFinish utterance: AVSpeechUtterance) {
            Task { @MainActor in owner?.finished() }
        }
        func speechSynthesizer(_ s: AVSpeechSynthesizer, didCancel utterance: AVSpeechUtterance) {
            Task { @MainActor in owner?.finished() }
        }
    }
}

private struct AILoadingDot: View {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @State private var isPulsing = false

    var body: some View {
        Circle()
            .fill(Color.primary.opacity(isPulsing ? 0.85 : 0.28))
            .frame(width: 7, height: 7)
            .scaleEffect(isPulsing ? 1.35 : 0.72)
            .frame(width: 18, height: 18)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.72).repeatForever(autoreverses: true),
                       value: isPulsing)
            .onAppear {
                guard !reduceMotion else { return }
                isPulsing = true
            }
            .accessibilityLabel("Assistant is thinking")
    }
}

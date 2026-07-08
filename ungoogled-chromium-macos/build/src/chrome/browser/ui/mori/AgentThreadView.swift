import SwiftUI

/// A full-page Codex agent thread — the assistant panel, full-screened. Shown
/// inside the web card when the active tab is an `.agent` tab (kicked off by
/// pressing `Tab` in the OmniBox). Bound to that tab's `CodexBrowserAssistant`,
/// which is owned by the tab model so the conversation survives tab switches.
///
/// Reuses the shared conversation surfaces (`AITranscript`, `AIComposer`,
/// `AIModelSelectors`, `AIInlineApprovalCard`) so it stays pixel-identical to
/// the side panel, just laid out as a centered reading column.
struct AgentThreadView: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var assistant: CodexBrowserAssistant
    @ObservedObject var tab: BrowserTab

    @Environment(\.palette) private var p
    @State private var draft: String = ""

    /// Comfortable reading column — chat reads wider than the settings form.
    private let contentWidth: CGFloat = 720

    var body: some View {
        VStack(spacing: 0) {
            header
            Hairline().opacity(0.6)

            if assistant.messages.isEmpty {
                emptyState
            } else {
                AITranscript(messages: assistant.messages,
                             maxContentWidth: contentWidth,
                             horizontalPadding: 24)
            }

            if let approval = assistant.pendingApproval {
                AIInlineApprovalCard(approval: approval,
                                     onAllow: { assistant.resolveApproval(true) },
                                     onDeny: { assistant.resolveApproval(false) })
                    .frame(maxWidth: contentWidth)
                    .frame(maxWidth: .infinity)
                    .padding(.horizontal, 24)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }

            AIComposer(text: $draft,
                       isWorking: assistant.isWorking,
                       onStop: { assistant.stop() }) { assistant.send($0) }
                .frame(maxWidth: contentWidth + 48)
                .frame(maxWidth: .infinity)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        // Opaque page surface, matching SettingsView — so the thread reads as a
        // real page regardless of the web-content suppression state behind it.
        .background(p.background.color)
        .animation(Motion.state, value: assistant.pendingApproval?.id)
        .task { await assistant.loadModelCatalogIfNeeded() }
    }

    private var header: some View {
        HStack(spacing: 10) {
            Icon(name: "sparkles", size: 16)
                .foregroundStyle(p.accent.color)

            VStack(alignment: .leading, spacing: 1) {
                Text(tab.title)
                    .font(Typography.ui(Typography.title, weight: .semibold))
                    .foregroundStyle(p.foreground.color)
                    .lineLimit(1)
                HStack(spacing: 6) {
                    Text(assistant.statusText)
                        .lineLimit(1)
                    if assistant.totalTokensUsed > 0 {
                        Text("|")
                        Text(assistant.tokenUsageLabel)
                            .lineLimit(1)
                    }
                }
                .font(Typography.ui(Typography.caption))
                .foregroundStyle(p.mutedForeground.color)
            }

            if assistant.isWorking {
                ProgressView()
                    .controlSize(.small)
                    .scaleEffect(0.7)
            }

            Spacer()

            AIModelSelectors(assistant: assistant)

            IconButton(systemName: "plus", size: 30) { store.presentLauncher() }
                .help("New task")
            IconButton(systemName: "xmark", size: 30) {
                store.closeTab(tab.id, allowFolderRemoval: true)
            }
            .help("Close agent")
        }
        .padding(.horizontal, 18)
        .frame(height: 56)
    }

    /// Defensive empty state — agent tabs normally launch with a first message,
    /// so this only appears if a thread is opened without a prompt.
    private var emptyState: some View {
        VStack(spacing: 14) {
            Icon(name: "sparkles", size: 34)
                .foregroundStyle(p.accent.color)
            Text("Give the agent a task")
                .font(Typography.ui(Typography.title, weight: .semibold))
                .foregroundStyle(p.foreground.color)
            Text("It can read your tabs and act across the browser for you.")
                .font(Typography.ui(Typography.small))
                .foregroundStyle(p.mutedForeground.color)
                .multilineTextAlignment(.center)
        }
        .padding(.horizontal, 24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

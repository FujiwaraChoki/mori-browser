import SwiftUI

struct CommandBar: View {
    var store: BrowserStore
    @State private var query = ""
    @State private var suggestions: [String] = []
    @State private var highlight = 0
    @FocusState private var focused: Bool

    private var hits: [CommandHit] {
        let all = CommandIndex.hits(query: query, workspace: store.workspace, suggestions: suggestions)
        let trimmed = query.trimmingCharacters(in: .whitespacesAndNewlines)
        guard trimmed.isEmpty else { return all }
        return all.filter { $0.section == .tabs || $0.section == .history }
    }

    var body: some View {
        VStack(spacing: 0) {
            field
            results
        }
        .frame(maxWidth: 640)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .background(Color(.systemBackground).ignoresSafeArea())
        .onAppear { focused = true }
        .onChange(of: query) { _, _ in highlight = 0 }
        .task(id: query) {
            let current = query
            try? await Task.sleep(for: .milliseconds(180))
            guard !Task.isCancelled else { return }
            suggestions = await SuggestionClient.fetch(current, engine: store.workspace.settings.searchEngine)
        }
    }

    private var field: some View {
        HStack(spacing: 12) {
            HStack(spacing: 8) {
                Image(systemName: "magnifyingglass")
                    .font(.system(size: 16, weight: .medium))
                    .foregroundStyle(.secondary)
                TextField("Search or enter address", text: $query)
                    .font(.body)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                    .keyboardType(.webSearch)
                    .focused($focused)
                    .submitLabel(.go)
                    .onSubmit(commitHighlighted)
                if !query.isEmpty {
                    Button {
                        query = ""
                        suggestions = []
                    } label: {
                        Image(systemName: "xmark.circle.fill")
                            .font(.system(size: 16))
                            .foregroundStyle(.tertiary)
                    }
                    .buttonStyle(.plain)
                    .accessibilityLabel("Clear")
                }
            }
            .padding(.horizontal, 12)
            .frame(height: 44)
            .background(Color(.tertiarySystemFill), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

            Button("Cancel") { store.commandBarPresented = false }
                .font(.body)
                .foregroundStyle(.primary)
        }
        .padding(.horizontal, 16)
        .padding(.top, 8)
        .padding(.bottom, 8)
    }

    private var results: some View {
        ScrollViewReader { proxy in
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 0) {
                    ForEach(Array(hits.enumerated()), id: \.element.id) { index, hit in
                        if index == 0 || hits[index - 1].section != hit.section {
                            Text(sectionTitle(hit.section))
                                .font(.system(size: 13, weight: .semibold))
                                .foregroundStyle(.secondary)
                                .padding(.horizontal, 20)
                                .padding(.top, index == 0 ? 12 : 22)
                                .padding(.bottom, 6)
                        }
                        Button {
                            store.commit(hit)
                        } label: {
                            HStack(alignment: .center, spacing: 14) {
                                Image(systemName: hit.symbol)
                                    .font(.system(size: 16, weight: .medium))
                                    .foregroundStyle(.secondary)
                                    .frame(width: 24, height: 24)
                                VStack(alignment: .leading, spacing: 2) {
                                    Text(hit.title)
                                        .font(.body)
                                        .foregroundStyle(.primary)
                                        .lineLimit(1)
                                    if let subtitle = hit.subtitle, !subtitle.isEmpty {
                                        Text(subtitle)
                                            .font(.subheadline)
                                            .foregroundStyle(.secondary)
                                            .lineLimit(1)
                                    }
                                }
                                Spacer(minLength: 0)
                            }
                            .padding(.horizontal, 16)
                            .padding(.vertical, 10)
                            .frame(maxWidth: .infinity, minHeight: 48, alignment: .leading)
                            .background {
                                if index == highlight, !query.isEmpty {
                                    RoundedRectangle(cornerRadius: 12, style: .continuous)
                                        .fill(Color(.secondarySystemFill))
                                        .padding(.horizontal, 8)
                                }
                            }
                            .contentShape(Rectangle())
                        }
                        .buttonStyle(.plain)
                        .id(hit.id)
                        .accessibilityAddTraits(index == highlight ? .isSelected : [])
                    }
                }
                .padding(.bottom, 24)
            }
            .scrollDismissesKeyboard(.interactively)
            .onChange(of: highlight) { _, index in
                guard hits.indices.contains(index) else { return }
                proxy.scrollTo(hits[index].id, anchor: .center)
            }
        }
    }

    private func sectionTitle(_ section: CommandSection) -> String {
        switch section {
        case .history: return "Recent"
        case .best: return "Top Hit"
        default: return section.rawValue
        }
    }

    private func commitHighlighted() {
        guard hits.indices.contains(highlight) else { return }
        store.commit(hits[highlight])
    }
}

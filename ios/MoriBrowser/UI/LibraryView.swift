import SwiftUI

struct LibraryView: View {
    var store: BrowserStore
    @Environment(\.dismiss) private var dismiss
    @State private var page = 0

    var body: some View {
        VStack(spacing: 12) {
            Picker("Library", selection: $page) {
                Text("Bookmarks").tag(0)
                Text("History").tag(1)
                Text("Downloads").tag(2)
            }
            .pickerStyle(.segmented)
            .padding(.horizontal, 16)
            .padding(.top, 8)
            Group {
                switch page {
                case 0: bookmarks
                case 1: history
                default: downloads
                }
            }
        }
        .navigationTitle("Library")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        .onAppear { store.refreshDownloads() }
    }

    private var bookmarks: some View {
        Group {
            if store.workspace.bookmarks.isEmpty {
                empty("No bookmarks yet.")
            } else {
                List {
                    ForEach(store.workspace.bookmarks) { bookmark in
                        Button {
                            open(bookmark.url)
                        } label: {
                            row(bookmark.title, subtitle: WebAddress.host(of: bookmark.url))
                        }
                        .swipeActions {
                            Button("Delete", role: .destructive) { store.removeBookmark(bookmark.id) }
                        }
                    }
                }
                .listStyle(.plain)
            }
        }
    }

    private var history: some View {
        Group {
            if store.workspace.history.isEmpty {
                empty("No history yet.")
            } else {
                List {
                    ForEach(store.workspace.history) { entry in
                        Button { open(entry.url) } label: {
                            row(entry.title, subtitle: WebAddress.host(of: entry.url))
                        }
                        .swipeActions {
                            Button("Delete", role: .destructive) { store.removeHistory(entry.id) }
                        }
                    }
                }
                .listStyle(.plain)
            }
        }
    }

    private var downloads: some View {
        Group {
            if store.downloads.isEmpty {
                empty("Downloads land in Mori's folder in the Files app.")
            } else {
                List(store.downloads, id: \.self) { name in
                    HStack {
                        Text(name).lineLimit(1)
                        Spacer()
                        ShareLink(item: DownloadDirectory.url.appendingPathComponent(name)) {
                            Image(systemName: "square.and.arrow.up")
                        }
                        .accessibilityLabel("Share \(name)")
                    }
                    .swipeActions {
                        Button("Delete", role: .destructive) { store.deleteDownload(name) }
                    }
                }
                .listStyle(.plain)
            }
        }
    }

    private func row(_ title: String, subtitle: String?) -> some View {
        VStack(alignment: .leading, spacing: 2) {
            Text(title).lineLimit(1).foregroundStyle(.primary)
            if let subtitle {
                Text(subtitle).font(.caption).foregroundStyle(.secondary).lineLimit(1)
            }
        }
    }

    private func empty(_ text: String) -> some View {
        Text(text)
            .font(.body)
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(24)
    }

    private func open(_ string: String) {
        guard let url = URL(string: string) else { return }
        store.open(url: url)
        dismiss()
    }
}

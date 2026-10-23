import SwiftUI

struct SettingsView: View {
    var store: BrowserStore
    @Environment(\.dismiss) private var dismiss
    @State private var confirmClear = false

    var body: some View {
        Form {
            Section("Search") {
                Picker("Search engine", selection: engine) {
                    ForEach(SearchEngine.allCases, id: \.self) { item in
                        Text(item.label).tag(item)
                    }
                }
            }
            Section {
                Button("Clear Browsing Data", role: .destructive) { confirmClear = true }
            } footer: {
                Text("Clears history, cookies, and cached files. Bookmarks and open tabs stay.")
            }
            Section("About") {
                LabeledContent("Version", value: version)
                Text("Mori on iPhone and iPad uses WebKit. Apple requires the system web engine for browsers on the App Store, so this app keeps favorites and tabs, and leaves the page to WebKit.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
            }
        }
        .navigationTitle("Settings")
        .navigationBarTitleDisplayMode(.inline)
        .toolbar {
            ToolbarItem(placement: .confirmationAction) {
                Button("Done") { dismiss() }
            }
        }
        .confirmationDialog("Clear browsing data?", isPresented: $confirmClear, titleVisibility: .visible) {
            Button("Clear", role: .destructive, action: store.clearBrowsingData)
            Button("Cancel", role: .cancel) {}
        } message: {
            Text("History, cookies, and cached files will be removed.")
        }
    }

    private var engine: Binding<SearchEngine> {
        Binding(
            get: { store.workspace.settings.searchEngine },
            set: { store.setSearchEngine($0) }
        )
    }

    private var version: String {
        let short = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String ?? "1.0"
        let build = Bundle.main.infoDictionary?["CFBundleVersion"] as? String ?? "1"
        return "\(short) (\(build))"
    }
}

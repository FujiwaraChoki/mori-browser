import SwiftUI

/// User-facing browser preferences. Persisted to `UserDefaults` and observed by
/// the chrome so changes apply live. One shared instance backs the single
/// window; views that need live updates observe it directly.
final class BrowserSettings: ObservableObject {
    static let shared = BrowserSettings()

    // MARK: General

    /// The page opened at launch and by "new tab → homepage".
    @Published var homepageURL: String {
        didSet { defaults.set(homepageURL, forKey: Key.homepage) }
    }

    /// What a freshly opened tab loads.
    @Published var newTabBehavior: NewTabBehavior {
        didSet { defaults.set(newTabBehavior.rawValue, forKey: Key.newTab) }
    }

    // MARK: Search

    @Published var searchEngine: SearchEngine {
        didSet { defaults.set(searchEngine.rawValue, forKey: Key.engine) }
    }

    /// Used only when `searchEngine == .custom`. `{query}` is substituted.
    @Published var customSearchTemplate: String {
        didSet { defaults.set(customSearchTemplate, forKey: Key.customEngine) }
    }

    // MARK: Privacy

    /// Blocks ad-serving requests using Mori's bundled Block List Project list.
    @Published var blockAds: Bool {
        didSet {
            defaults.set(blockAds, forKey: Key.blockAds)
            MoriBrowserView.setAdBlockerEnabled(blockAds)
        }
    }

    // MARK: Appearance

    @Published var theme: ThemePreference {
        didSet { defaults.set(theme.rawValue, forKey: Key.theme) }
    }

    /// Whether the tab sidebar is shown when the window opens.
    @Published var showSidebarOnLaunch: Bool {
        didSet { defaults.set(showSidebarOnLaunch, forKey: Key.sidebarOnLaunch) }
    }

    /// Which side of the window hosts the tab sidebar.
    @Published var sidebarPosition: SidebarPosition {
        didSet { defaults.set(sidebarPosition.rawValue, forKey: Key.sidebarPosition) }
    }

    /// The user's custom gradient theme (chrome wash + derived accent). Empty
    /// means "no custom theme" — the chrome uses the plain light/dark tint.
    /// Persisted as JSON. Single global theme for now; when multi-space lands,
    /// this becomes a per-space map keyed by the active space.
    @Published var gradientTheme: GradientTheme {
        didSet {
            if let data = try? JSONEncoder().encode(gradientTheme) {
                defaults.set(data, forKey: Key.gradientTheme)
            }
        }
    }

    // MARK: Media

    /// Automatically enter Picture-in-Picture when you switch away from a tab
    /// that's playing video (YouTube, etc.).
    @Published var autoPiP: Bool {
        didSet {
            defaults.set(autoPiP, forKey: Key.autoPiP)
            MoriBrowserView.setAutoPiPEnabled(autoPiP)
        }
    }

    // MARK: Resolution helpers

    /// The built-in start page, served from Mori's internal scheme so it
    /// reads as native chrome (empty address bar, no file:// path).
    static let defaultHomepageURL = "mori://newtab/"

    /// Build the destination for a query, honoring the active engine.
    func searchURL(for query: String) -> String {
        var allowed = CharacterSet.urlQueryAllowed
        allowed.remove(charactersIn: "&=+?/#")
        let encoded = query.addingPercentEncoding(withAllowedCharacters: allowed) ?? query
        let rawTemplate = searchEngine == .custom ? customSearchTemplate : searchEngine.queryTemplate
        let template = rawTemplate.isEmpty ? SearchEngine.google.queryTemplate : rawTemplate
        if template.contains("{query}") {
            return template.replacingOccurrences(of: "{query}", with: encoded)
        }
        // Tolerate a bare endpoint by appending the query.
        return template + encoded
    }

    /// The URL a new tab should load given the current behavior setting.
    var newTabURL: String {
        switch newTabBehavior {
        case .homepage: return homepageURL
        case .blank: return "about:blank"
        }
    }

    // MARK: Persistence

    private let defaults: UserDefaults

    private enum Key {
        static let homepage = "mori.homepageURL"
        static let newTab = "mori.newTabBehavior"
        static let engine = "mori.searchEngine"
        static let customEngine = "mori.customSearchTemplate"
        static let blockAds = "mori.blockAds"
        static let theme = "mori.theme"
        static let sidebarOnLaunch = "mori.showSidebarOnLaunch"
        static let sidebarPosition = "mori.sidebarPosition"
        static let autoPiP = "mori.autoPiP"
        static let gradientTheme = "mori.gradientTheme"
    }

    init(defaults: UserDefaults = .standard) {
        self.defaults = defaults
        // Migrate the earlier bundled file:// home page to the internal scheme.
        let storedHome = defaults.string(forKey: Key.homepage)
        let resolvedHome: String
        if let storedHome, storedHome.contains("home.html") {
            resolvedHome = BrowserSettings.defaultHomepageURL
            defaults.set(resolvedHome, forKey: Key.homepage)
        } else {
            resolvedHome = storedHome ?? BrowserSettings.defaultHomepageURL
        }
        homepageURL = resolvedHome
        newTabBehavior = NewTabBehavior(rawValue: defaults.string(forKey: Key.newTab) ?? "")
            ?? .homepage
        searchEngine = SearchEngine(rawValue: defaults.string(forKey: Key.engine) ?? "")
            ?? .google
        customSearchTemplate = defaults.string(forKey: Key.customEngine)
            ?? "https://www.example.com/search?q={query}"
        blockAds = defaults.object(forKey: Key.blockAds) as? Bool ?? true
        theme = ThemePreference(rawValue: defaults.string(forKey: Key.theme) ?? "")
            ?? .system
        // Default the sidebar on (matches the Mori default chrome).
        showSidebarOnLaunch = defaults.object(forKey: Key.sidebarOnLaunch) as? Bool ?? true
        // Default the sidebar to the right to preserve the existing layout.
        sidebarPosition = SidebarPosition(rawValue: defaults.string(forKey: Key.sidebarPosition) ?? "")
            ?? .right
        gradientTheme = defaults.data(forKey: Key.gradientTheme)
            .flatMap { try? JSONDecoder().decode(GradientTheme.self, from: $0) }
            ?? .none
        autoPiP = defaults.object(forKey: Key.autoPiP) as? Bool ?? true

        // Apply the persisted auto-PiP default to the engine on startup.
        MoriBrowserView.setAutoPiPEnabled(autoPiP)
        MoriBrowserView.setAdBlockerEnabled(blockAds)
    }
}

// MARK: - Option enums

enum SidebarPosition: String, CaseIterable, Identifiable {
    case left, right
    var id: String { rawValue }

    var label: String {
        switch self {
        case .left: return "Left"
        case .right: return "Right"
        }
    }

    var symbol: String {
        switch self {
        case .left: return "sidebar.left"
        case .right: return "sidebar.right"
        }
    }

    var edge: Edge {
        switch self {
        case .left: return .leading
        case .right: return .trailing
        }
    }
}

enum ThemePreference: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }

    var label: String {
        switch self {
        case .system: return "System"
        case .light: return "Light"
        case .dark: return "Dark"
        }
    }

    var symbol: String {
        switch self {
        case .system: return "circle.lefthalf.filled"
        case .light: return "sun.max"
        case .dark: return "moon"
        }
    }

    /// nil = follow the system appearance.
    var colorScheme: ColorScheme? {
        switch self {
        case .system: return nil
        case .light: return .light
        case .dark: return .dark
        }
    }
}

enum NewTabBehavior: String, CaseIterable, Identifiable {
    case homepage, blank
    var id: String { rawValue }

    var label: String {
        switch self {
        case .homepage: return "Open homepage"
        case .blank: return "Open a blank page"
        }
    }
}

enum SearchEngine: String, CaseIterable, Identifiable {
    case google, duckduckgo, bing, brave, custom
    var id: String { rawValue }

    var label: String {
        switch self {
        case .google: return "Google"
        case .duckduckgo: return "DuckDuckGo"
        case .bing: return "Bing"
        case .brave: return "Brave"
        case .custom: return "Custom…"
        }
    }

    /// `{query}` is replaced with the percent-encoded search terms.
    var queryTemplate: String {
        switch self {
        case .google: return "https://www.google.com/search?q={query}"
        case .duckduckgo: return "https://duckduckgo.com/?q={query}"
        case .bing: return "https://www.bing.com/search?q={query}"
        case .brave: return "https://search.brave.com/search?q={query}"
        case .custom: return ""
        }
    }
}

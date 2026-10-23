import SwiftUI

struct RootView: View {
    @Bindable var store: BrowserStore
    @Environment(\.colorScheme) private var systemScheme
    @Environment(\.horizontalSizeClass) private var sizeClass
    @Environment(\.scenePhase) private var scenePhase
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var scheme: ColorScheme { store.chromeScheme(system: systemScheme) }
    private var palette: MoriPalette { MoriPalette.resolve(theme: nil, scheme: scheme) }

    var body: some View {
        Group {
            if isPad {
                padLayout
            } else {
                phoneLayout
            }
        }
        .environment(\.moriPalette, palette)
        .environment(\.colorScheme, scheme)
        .preferredColorScheme(store.preferredScheme)
        .tint(palette.primary)
        .overlay {
            if store.commandBarPresented {
                CommandBar(store: store)
                    .id(store.commandToken)
            }
        }
        .overlay(alignment: .bottom) {
            if let toast = store.toast {
                ToastPill(text: toast)
                    .padding(.bottom, sizeClass == .regular ? 28 : 78)
            }
        }
        .sheet(item: $store.pageSheet) { destination in
            NavigationStack {
                DestinationScreen(store: store, destination: destination)
            }
        }
        .sheet(item: $store.sharedLink) { link in
            ActivityView(items: [link.url])
        }
        .fullScreenCover(item: $store.readerArticle) { article in
            ReaderView(article: article)
        }
        .fullScreenCover(isPresented: onboarding) {
            OnboardingView { store.finishOnboarding() }
        }
        .modifier(PageDialogModifier(store: store))
        .onAppear { store.ensureSelectedSession() }
        .onChange(of: store.selectedTabID) { _, _ in store.ensureSelectedSession() }
        .onChange(of: scenePhase) { _, phase in
            if phase == .background { store.saveNow() }
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: store.commandBarPresented)
    }

    private var isPad: Bool { UIDevice.current.userInterfaceIdiom == .pad }

    private var padLayout: some View {
        HStack(spacing: 0) {
            SidebarView(store: store, embedded: false)
                .frame(width: 300)
            ZStack(alignment: .bottom) {
                PageColumn(store: store, showsCard: true)
                ArcBar(store: store, embedded: false, showsTabs: false)
                    .padding(.horizontal, 16)
                    .padding(.bottom, 12)
                    .modifier(CollapsingBar(hidden: store.bottomBarHidden, reduceMotion: reduceMotion))
            }
            .clipShape(RoundedRectangle(cornerRadius: 22, style: .continuous))
            .padding(10)
        }
        .background(SpaceCanvas(theme: nil).ignoresSafeArea())
    }

    private var phoneLayout: some View {
        ZStack(alignment: .bottom) {
            PageColumn(store: store, showsCard: false)
                .ignoresSafeArea()
            ArcBar(store: store, embedded: false, showsTabs: true)
                .padding(.horizontal, 14)
                .padding(.bottom, 8)
                .safeAreaPadding(.bottom)
                .simultaneousGesture(revealTabs)
                .modifier(CollapsingBar(hidden: store.bottomBarHidden, reduceMotion: reduceMotion))
        }
        .sheet(isPresented: $store.sidebarPresented) {
            SidebarView(store: store, embedded: false)
                .presentationDetents([.medium, .large])
                .presentationDragIndicator(.visible)
                .presentationCornerRadius(28)
        }
    }

    private var revealTabs: some Gesture {
        DragGesture(minimumDistance: 18, coordinateSpace: .local)
            .onEnded { value in
                if value.translation.height < -28, abs(value.translation.width) < 80 {
                    store.sidebarPresented = true
                }
            }
    }

    private var onboarding: Binding<Bool> {
        Binding(
            get: { !store.workspace.hasOnboarded },
            set: { if !$0 { store.finishOnboarding() } }
        )
    }
}

private struct CollapsingBar: ViewModifier {
    var hidden: Bool
    var reduceMotion: Bool

    func body(content: Content) -> some View {
        content
            .offset(y: hidden ? 160 : 0)
            .allowsHitTesting(!hidden)
            .animation(reduceMotion ? nil : .spring(response: 0.34, dampingFraction: 0.88), value: hidden)
    }
}

struct DestinationScreen: View {
    var store: BrowserStore
    var destination: Destination

    var body: some View {
        switch destination {
        case .settings:
            SettingsView(store: store)
        case .library:
            LibraryView(store: store)
        case .space(let id):
            SpaceEditor(store: store, spaceID: id)
        }
    }
}

struct PageColumn: View {
    var store: BrowserStore
    var showsCard: Bool
    @Environment(\.moriPalette) private var palette

    var body: some View {
        ZStack {
            if store.addressIsPlaceholder {
                NewTabPage(store: store)
            } else if let session = store.activeSession {
                WebContainer(webView: session.webView, isLoading: session.isLoading)
                if let failure = session.failure {
                    ErrorPage(message: failure) { session.reload() }
                }
            } else {
                ProgressView()
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(pageBackground)
        .overlay(alignment: .top) {
            progress.safeAreaPadding(.top)
        }
        .safeAreaInset(edge: .top, spacing: 0) {
            if store.findPresented {
                FindBar(store: store)
            }
        }
    }

    private var pageBackground: Color {
        if store.addressIsPlaceholder { return palette.background }
        return showsCard ? palette.card : Color(.systemBackground)
    }

    @ViewBuilder
    private var progress: some View {
        if let session = store.activeSession, session.isLoading, !store.addressIsPlaceholder {
            GeometryReader { geo in
                Rectangle()
                    .fill(palette.primary)
                    .frame(width: max(2, geo.size.width * session.progress), height: 2)
            }
            .frame(height: 2)
            .accessibilityHidden(true)
        }
    }
}

struct NewTabPage: View {
    var store: BrowserStore

    var body: some View {
        ScrollView {
            if !store.workspace.favorites.isEmpty {
                FavoriteGrid(store: store, large: true)
                    .padding(.horizontal, 22)
                    .padding(.top, 72)
                    .padding(.bottom, 108)
            }
        }
        .scrollBounceBehavior(.basedOnSize)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }
}

struct ErrorPage: View {
    let message: String
    var retry: () -> Void
    @Environment(\.moriPalette) private var palette

    var body: some View {
        VStack(spacing: 12) {
            Text("Can't open this page")
                .font(.headline)
                .foregroundStyle(palette.foreground)
            Text(message)
                .font(.subheadline)
                .foregroundStyle(palette.mutedForeground)
                .multilineTextAlignment(.center)
            Button("Try Again", action: retry)
                .buttonStyle(.borderedProminent)
        }
        .padding(24)
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .background(palette.card)
    }
}

import XCTest
@testable import MoriBrowser

final class MoriBrowserTests: XCTestCase {
    func testFreshWorkspaceHasOneEmptyTab() {
        let workspace = Workspace.fresh()
        XCTAssertEqual(workspace.spaces.count, 1)
        XCTAssertEqual(workspace.spaces[0].name, "Personal")
        XCTAssertFalse(workspace.hasOnboarded)
        XCTAssertEqual(workspace.tabs.count, 1)
        XCTAssertEqual(workspace.activeSpace.selectedTabID, workspace.activeSpace.today.first)
        XCTAssertNil(workspace.tabs[workspace.activeSpace.selectedTabID!]?.url)
    }

    func testOmniboxResolvesAddressesAndSearches() {
        let engine = SearchEngine.google
        XCTAssertNil(Omnibox.resolve("   ", engine: engine))
        guard case .url(let secure) = Omnibox.resolve("https://example.com/docs", engine: engine) else {
            return XCTFail("Expected a URL")
        }
        XCTAssertEqual(secure.absoluteString, "https://example.com/docs")
        guard case .url(let host) = Omnibox.resolve("example.com", engine: engine) else {
            return XCTFail("Expected a URL")
        }
        XCTAssertEqual(host.absoluteString, "https://example.com")
        guard case .url(let local) = Omnibox.resolve("localhost:8080/app", engine: engine) else {
            return XCTFail("Expected a local URL")
        }
        XCTAssertEqual(local.absoluteString, "http://localhost:8080/app")
        guard case .url(let lan) = Omnibox.resolve("192.168.1.20", engine: engine) else {
            return XCTFail("Expected a private IP")
        }
        XCTAssertEqual(lan.scheme, "http")
        guard case .url(let publicIP) = Omnibox.resolve("1.1.1.1", engine: engine) else {
            return XCTFail("Expected a public IP")
        }
        XCTAssertEqual(publicIP.scheme, "https")
        guard case .search(let query, let search) = Omnibox.resolve("mori browser", engine: engine) else {
            return XCTFail("Expected a search")
        }
        XCTAssertEqual(query, "mori browser")
        XCTAssertEqual(search.host, "www.google.com")
        XCTAssertTrue(search.query?.contains("mori") == true)
        guard case .search = Omnibox.resolve("javascript:alert(1)", engine: engine) else {
            return XCTFail("javascript: must not navigate")
        }
        guard case .search = Omnibox.resolve("file:///etc/passwd", engine: engine) else {
            return XCTFail("file: must not navigate")
        }
    }

    func testCloseLastTabLeavesAFreshOne() {
        var workspace = Workspace.fresh()
        let original = workspace.activeSpace.selectedTabID!
        let closed = workspace.close(tab: original)
        XCTAssertEqual(closed?.record.id, original)
        XCTAssertEqual(workspace.tabs.count, 1)
        XCTAssertNotEqual(workspace.activeSpace.selectedTabID, original)
        XCTAssertEqual(workspace.tabs[workspace.activeSpace.selectedTabID!]?.title, "New Tab")
    }

    func testCloseSelectsThePreviousTab() {
        var workspace = Workspace.fresh()
        let first = workspace.activeSpace.selectedTabID!
        let second = workspace.newTab(in: workspace.activeSpaceID)
        let third = workspace.newTab(in: workspace.activeSpaceID)
        XCTAssertEqual(workspace.activeSpace.selectedTabID, third)
        _ = workspace.close(tab: third)
        XCTAssertEqual(workspace.activeSpace.selectedTabID, second)
        XCTAssertTrue(workspace.activeSpace.today.contains(first))
    }

    func testFavoriteMovesATabAndStopsAtTwelve() {
        var workspace = Workspace.fresh()
        var ids: [UUID] = []
        for index in 0..<12 {
            let id = workspace.newTab(in: workspace.activeSpaceID)
            workspace.navigate(tab: id, to: URL(string: "https://example.com/\(index)")!)
            XCTAssertTrue(workspace.favorite(tab: id))
            ids.append(id)
        }
        XCTAssertEqual(workspace.favorites.count, 12)
        XCTAssertFalse(workspace.activeSpace.today.contains(ids[0]))
        let extra = workspace.newTab(in: workspace.activeSpaceID)
        XCTAssertFalse(workspace.favorite(tab: extra))
        XCTAssertTrue(workspace.activeSpace.today.contains(extra))
    }

    func testPinRemembersTheURL() {
        var workspace = Workspace.fresh()
        let id = workspace.activeSpace.selectedTabID!
        workspace.navigate(tab: id, to: URL(string: "https://example.com/pin")!)
        workspace.pin(tab: id)
        XCTAssertEqual(workspace.tabs[id]?.pinnedURL, "https://example.com/pin")
        XCTAssertFalse(workspace.activeSpace.today.contains(id))
        XCTAssertTrue(workspace.isPinned(id))
        workspace.unpin(tab: id)
        XCTAssertTrue(workspace.activeSpace.today.contains(id))
    }

    func testClearTodayLeavesOneEmptyTab() {
        var workspace = Workspace.fresh()
        let pinned = workspace.newTab(in: workspace.activeSpaceID)
        workspace.navigate(tab: pinned, to: URL(string: "https://example.com/keep")!)
        workspace.pin(tab: pinned)
        _ = workspace.newTab(in: workspace.activeSpaceID)
        _ = workspace.newTab(in: workspace.activeSpaceID)
        let closed = workspace.clearToday(in: workspace.activeSpaceID)
        XCTAssertFalse(closed.isEmpty)
        XCTAssertEqual(workspace.activeSpace.today.count, 1)
        XCTAssertNil(workspace.tabs[workspace.activeSpace.today[0]]?.url)
        XCTAssertTrue(workspace.isPinned(pinned))
    }

    func testCannotDeleteTheLastNormalSpace() {
        var workspace = Workspace.fresh()
        XCTAssertFalse(workspace.deleteSpace(workspace.activeSpaceID))
        let second = workspace.addSpace(private: false)
        XCTAssertTrue(workspace.deleteSpace(second))
        XCTAssertEqual(workspace.spaces.count, 1)
        XCTAssertEqual(workspace.activeSpace.name, "Personal")
    }

    func testSnapshotDropsPrivateSpaces() throws {
        var workspace = Workspace.fresh()
        let privateID = workspace.addSpace(private: true)
        let tab = try XCTUnwrap(workspace.activeSpace.selectedTabID)
        workspace.navigate(tab: tab, to: URL(string: "https://secret.example/path")!)
        let snapshot = workspace.snapshot()
        XCTAssertFalse(snapshot.spaces.contains { $0.id == privateID })
        XCTAssertFalse(snapshot.tabs.contains { $0.url?.contains("secret.example") == true })
        XCTAssertTrue(workspace.spaces.contains { $0.id == privateID })
    }

    func testHistoryDedupesAndCaps() {
        var workspace = Workspace.fresh()
        workspace.recordHistory(title: "One", url: "https://example.com/a")
        workspace.recordHistory(title: "Two", url: "https://example.com/b")
        workspace.recordHistory(title: "One again", url: "https://example.com/a")
        XCTAssertEqual(workspace.history.first?.url, "https://example.com/a")
        XCTAssertEqual(workspace.history.first?.title, "One again")
        XCTAssertEqual(workspace.history.count, 2)
        for index in 0..<(Workspace.maxHistory + 5) {
            workspace.recordHistory(title: "\(index)", url: "https://example.com/n\(index)")
        }
        XCTAssertEqual(workspace.history.count, Workspace.maxHistory)
    }

    func testReopenRestoresAClosedTab() {
        var workspace = Workspace.fresh()
        let id = workspace.activeSpace.selectedTabID!
        workspace.navigate(tab: id, to: URL(string: "https://example.com/back")!)
        let closed = workspace.close(tab: id)
        XCTAssertNotNil(closed)
        workspace.reopen(closed!)
        XCTAssertEqual(workspace.tabs[id]?.url, "https://example.com/back")
        XCTAssertEqual(workspace.activeSpace.selectedTabID, id)
    }

    func testMoveIntoAFolder() {
        var workspace = Workspace.fresh()
        let id = workspace.activeSpace.selectedTabID!
        let folder = workspace.addFolder(named: "Later", in: workspace.activeSpaceID)!
        XCTAssertTrue(workspace.move(tab: id, toFolder: folder))
        guard case .folder(let id) = workspace.placement(of: id)?.kind else {
            return XCTFail("Expected the tab inside the folder")
        }
        XCTAssertEqual(id, folder)
    }

    func testSuggestionParser() {
        let google = Data("[\"rust\", [\"rust lang\", \"rustup\"]]".utf8)
        XCTAssertEqual(SuggestionParser.parse(google), ["rust lang", "rustup"])
        let phrases = Data("[{\"phrase\":\"alpha\"},{\"phrase\":\"alpha\"},{\"phrase\":\"beta\"}]".utf8)
        XCTAssertEqual(SuggestionParser.parse(phrases), ["alpha", "beta"])
    }

    func testCommandIndexRanksAnAddressFirst() {
        var workspace = Workspace.fresh()
        let id = workspace.activeSpace.selectedTabID!
        workspace.navigate(tab: id, to: URL(string: "https://example.com")!)
        workspace.tabs[id]?.title = "Example"
        let hits = CommandIndex.hits(query: "example.com", workspace: workspace, suggestions: ["example com"])
        XCTAssertEqual(hits.first?.section, .best)
        guard case .navigate = hits.first?.kind else {
            return XCTFail("Expected the address to lead")
        }
        XCTAssertTrue(hits.contains { if case .tab = $0.kind { return true } else { return false } })
    }

    func testSessionRoundTripOmitsPrivateData() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let store = SessionStore(directory: directory)
        var workspace = Workspace.fresh()
        workspace.hasOnboarded = true
        _ = workspace.addSpace(private: true)
        let tab = try XCTUnwrap(workspace.activeSpace.selectedTabID)
        workspace.navigate(tab: tab, to: URL(string: "https://secret.example/hide")!)
        workspace.rename(space: workspace.spaces[0].id, to: "Home")
        store.save(workspace)
        let data = try Data(contentsOf: directory.appendingPathComponent("session.json"))
        XCTAssertFalse(String(decoding: data, as: UTF8.self).contains("secret.example"))
        let loaded = store.load()
        XCTAssertTrue(loaded.hasOnboarded)
        XCTAssertEqual(loaded.spaces.first?.name, "Home")
        XCTAssertFalse(loaded.spaces.contains { $0.isPrivate })
    }

    func testBrokenSessionStartsFresh() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        try Data("not json".utf8).write(to: directory.appendingPathComponent("session.json"))
        let loaded = SessionStore(directory: directory).load()
        XCTAssertEqual(loaded.spaces.first?.name, "Personal")
        XCTAssertTrue(FileManager.default.fileExists(atPath: directory.appendingPathComponent("session.json.broken").path))
    }

    func testDefaultAccentIsBlue() {
        let (red, green, blue) = MoriColor.srgb(lightness: 0.645, chroma: 0.11, hue: 241.2)
        XCTAssertGreaterThan(blue, red)
        XCTAssertGreaterThan(blue, green)
        XCTAssertGreaterThan(blue, 0.45)
    }
}

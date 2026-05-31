import Foundation

struct CodexModelOption: Identifiable, Equatable {
    let id: String
    let displayName: String
    let defaultReasoningEffort: String
    let reasoningEfforts: [CodexReasoningEffortOption]
    let isDefault: Bool
}

struct CodexReasoningEffortOption: Identifiable, Equatable {
    let id: String
    let displayName: String
    let description: String
}

struct CodexConversationSummary: Identifiable, Equatable {
    let id: String
    let title: String
    let preview: String
    let updatedAt: Date
}

@MainActor
final class CodexBrowserAssistant: ObservableObject {
    @Published var messages: [AIMessage] = []
    @Published var statusText: String = "Local Codex"
    @Published var isWorking: Bool = false
    @Published var isLoadingModels: Bool = false
    @Published var modelOptions: [CodexModelOption] = []
    @Published var selectedModelID: String = "" {
        didSet {
            guard oldValue != selectedModelID else { return }
            updateReasoningEffortsForSelectedModel()
        }
    }
    @Published var reasoningEffortOptions: [CodexReasoningEffortOption] = []
    @Published var selectedReasoningEffort: String = ""
    @Published var conversationHistory: [CodexConversationSummary] = []
    @Published var isLoadingHistory: Bool = false
    @Published var historyError: String?

    private weak var store: BrowserStore?
    // Enabled by default; opt out by launching with MORI_ENABLE_CODEX_ASSISTANT=0.
    private let isEnabled = ProcessInfo.processInfo.environment["MORI_ENABLE_CODEX_ASSISTANT"] != "0"
    private let connection = CodexAppServerConnection()
    private var threadId: String?
    private var activeAssistantMessageId: AIMessage.ID?
    private var usesDynamicTools = true
    private var fallbackToolIterations = 0
    private var pendingAssistantText = ""
    private var turnWatchdogTask: Task<Void, Never>?

    init(store: BrowserStore) {
        self.store = store
        if !isEnabled {
            statusText = "Disabled"
        }
        connection.onNotification = { [weak self] method, params in
            Task { @MainActor in self?.handleNotification(method: method, params: params) }
        }
        connection.onServerRequest = { [weak self] method, params in
            await self?.handleServerRequest(method: method, params: params) ?? [:]
        }
    }

    func loadModelCatalogIfNeeded() async {
        guard isEnabled else { return }
        guard modelOptions.isEmpty, !isLoadingModels else { return }
        isLoadingModels = true
        defer { isLoadingModels = false }
        do {
            try await refreshModelCatalog()
        } catch {
            if statusText == "Local Codex" {
                statusText = "Models unavailable"
            }
        }
    }

    func loadConversationHistory(searchTerm: String = "") async {
        guard isEnabled else {
            historyError = "Set MORI_ENABLE_CODEX_ASSISTANT=1 before launching Mori to enable Codex history."
            conversationHistory = []
            return
        }
        isLoadingHistory = true
        historyError = nil
        defer { isLoadingHistory = false }
        do {
            try await connection.connectIfNeeded()
            var params: [String: Any] = [
                "limit": 40,
                "sortKey": "updated_at",
                "sortDirection": "desc",
                "sourceKinds": ["appServer"],
                "archived": false,
                "cwd": NSHomeDirectory()
            ]
            let trimmedSearch = searchTerm.trimmingCharacters(in: .whitespacesAndNewlines)
            if !trimmedSearch.isEmpty {
                params["searchTerm"] = trimmedSearch
            }
            let response = try await withTimeout(seconds: 8) {
                try await self.connection.request(method: "thread/list", params: params)
            }
            conversationHistory = parseConversationHistory(from: response)
        } catch {
            historyError = error.localizedDescription
            conversationHistory = []
        }
    }

    func openConversation(_ conversation: CodexConversationSummary) async {
        guard isEnabled else {
            historyError = "Set MORI_ENABLE_CODEX_ASSISTANT=1 before launching Mori to enable Codex."
            return
        }
        guard !isWorking else { return }
        isLoadingHistory = true
        historyError = nil
        statusText = "Loading History"
        defer {
            isLoadingHistory = false
            statusText = "Local Codex"
        }
        do {
            let response = try await withTimeout(seconds: 8) {
                try await self.connection.request(
                    method: "thread/read",
                    params: ["threadId": conversation.id, "includeTurns": true]
                )
            }
            let loadedMessages = messagesFromThreadRead(response)
            messages = loadedMessages.isEmpty
                ? [AIMessage(role: .assistant, text: "No visible messages in this conversation.")]
                : loadedMessages
            activeAssistantMessageId = nil
            pendingAssistantText = ""
            fallbackToolIterations = 0
            threadId = conversation.id
            usesDynamicTools = false
            try? await resumeConversation(conversation.id)
        } catch {
            historyError = error.localizedDescription
        }
    }

    func send(_ rawText: String) {
        let text = rawText.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty, !isWorking else { return }
        guard isEnabled else {
            messages.append(AIMessage(role: .user, text: text))
            messages.append(AIMessage(
                role: .assistant,
                text: "Mori's local Codex assistant is disabled. It's enabled by default; relaunch without MORI_ENABLE_CODEX_ASSISTANT=0 to grant the local Codex app server browser-assistant access again."
            ))
            statusText = "Disabled"
            return
        }
        messages.append(AIMessage(role: .user, text: text))
        let placeholder = AIMessage(role: .assistant, text: "")
        messages.append(placeholder)
        activeAssistantMessageId = placeholder.id
        turnWatchdogTask?.cancel()
        isWorking = true
        statusText = "Starting Codex"

        Task { @MainActor in
            do {
                let threadId = try await ensureThread()
                fallbackToolIterations = 0
                pendingAssistantText = ""
                let prompt = try await promptForUserRequest(text)
                try await startTurn(threadId: threadId, prompt: prompt)
            } catch {
                replaceActiveAssistantText("I couldn't reach the local Codex app server: \(error.localizedDescription)")
                isWorking = false
                statusText = "Disconnected"
            }
        }
    }

    private func ensureThread() async throws -> String {
        if let threadId { return threadId }
        try await connection.connectIfNeeded()
        if modelOptions.isEmpty {
            try? await refreshModelCatalog()
        }
        var baseParams: [String: Any] = [
                "cwd": NSHomeDirectory(),
                "approvalPolicy": "never",
                "sandbox": "danger-full-access",
                "personality": "friendly",
                "serviceName": "mori_browser"
        ]
        if !selectedModelID.isEmpty {
            baseParams["model"] = selectedModelID
        }
        let result: [String: Any]
        if ProcessInfo.processInfo.environment["MORI_CODEX_DYNAMIC_TOOLS"] == "1" {
            let dynamicParams = baseParams.merging(["dynamicTools": BrowserAutomation.dynamicTools]) { _, new in new }
            result = try await withTimeout(seconds: 8) {
                try await self.connection.request(method: "thread/start", params: dynamicParams)
            }
            usesDynamicTools = true
        } else {
            result = try await connection.request(method: "thread/start", params: baseParams)
            usesDynamicTools = false
        }
        guard let thread = result["thread"] as? [String: Any],
              let id = thread["id"] as? String
        else {
            throw CodexAppServerError.protocolError("thread/start did not return a thread id.")
        }
        threadId = id
        statusText = "Connected"
        return id
    }

    private func startTurn(threadId: String, prompt: String) async throws {
        statusText = "Thinking"
        var params: [String: Any] = [
            "threadId": threadId,
            "input": [["type": "text", "text": prompt]],
            "approvalPolicy": "never",
            "sandboxPolicy": ["type": "dangerFullAccess"]
        ]
        if !selectedModelID.isEmpty {
            params["model"] = selectedModelID
        }
        if !selectedReasoningEffort.isEmpty {
            params["effort"] = selectedReasoningEffort
        }
        _ = try await withTimeout(seconds: 15) {
            try await self.connection.request(method: "turn/start", params: params)
        }
        armTurnWatchdog()
    }

    private func refreshModelCatalog() async throws {
        try await connection.connectIfNeeded()
        let response = try await withTimeout(seconds: 8) {
            try await self.connection.request(method: "model/list", params: [:])
        }
        let parsed = parseModelCatalog(from: response)
        guard !parsed.isEmpty else { return }
        let previousModel = selectedModelID
        let previousEffort = selectedReasoningEffort
        modelOptions = parsed

        if parsed.contains(where: { $0.id == previousModel }) {
            selectedModelID = previousModel
            updateReasoningEffortsForSelectedModel(preferredEffort: previousEffort)
            return
        }

        let defaultModel = parsed.first(where: \.isDefault) ?? parsed[0]
        selectedModelID = defaultModel.id
        updateReasoningEffortsForSelectedModel(preferredEffort: defaultModel.defaultReasoningEffort)
    }

    private func parseModelCatalog(from response: [String: Any]) -> [CodexModelOption] {
        guard let data = response["data"] as? [[String: Any]] else { return [] }
        return data.compactMap { raw in
            let hidden = raw["hidden"] as? Bool ?? false
            guard !hidden else { return nil }
            let model = raw["model"] as? String ?? raw["id"] as? String ?? ""
            guard !model.isEmpty else { return nil }
            let efforts = reasoningEfforts(from: raw["supportedReasoningEfforts"])
            let defaultEffort = raw["defaultReasoningEffort"] as? String
            return CodexModelOption(
                id: model,
                displayName: raw["displayName"] as? String ?? model,
                defaultReasoningEffort: defaultEffort ?? efforts.first?.id ?? "",
                reasoningEfforts: efforts,
                isDefault: raw["isDefault"] as? Bool ?? false
            )
        }
    }

    private func parseConversationHistory(from response: [String: Any]) -> [CodexConversationSummary] {
        guard let data = response["data"] as? [[String: Any]] else { return [] }
        return data.compactMap { raw in
            guard let id = raw["id"] as? String, !id.isEmpty else { return nil }
            let preview = raw["preview"] as? String ?? ""
            let title = conversationTitle(name: raw["name"] as? String, preview: preview)
            let updatedAt = number(raw["updatedAt"]) ?? number(raw["createdAt"]) ?? 0
            return CodexConversationSummary(
                id: id,
                title: title,
                preview: cleanConversationPreview(preview),
                updatedAt: Date(timeIntervalSince1970: updatedAt)
            )
        }
    }

    private func conversationTitle(name: String?, preview: String) -> String {
        if let name = name?.trimmingCharacters(in: .whitespacesAndNewlines), !name.isEmpty {
            return name
        }
        let cleaned = cleanVisibleUserText(preview) ?? cleanConversationPreview(preview)
        if cleaned.isEmpty { return "Untitled Conversation" }
        return String(cleaned.prefix(80))
    }

    private func cleanConversationPreview(_ preview: String) -> String {
        let cleaned = (cleanVisibleUserText(preview) ?? preview)
            .replacingOccurrences(of: "\\s+", with: " ", options: .regularExpression)
            .trimmingCharacters(in: .whitespacesAndNewlines)
        return String(cleaned.prefix(120))
    }

    private func messagesFromThreadRead(_ response: [String: Any]) -> [AIMessage] {
        guard let thread = response["thread"] as? [String: Any],
              let turns = thread["turns"] as? [[String: Any]]
        else { return [] }
        var loaded: [AIMessage] = []
        for turn in turns {
            guard let items = turn["items"] as? [[String: Any]] else { continue }
            for item in items {
                switch item["type"] as? String {
                case "userMessage":
                    guard let text = visibleUserText(from: item), !text.isEmpty else { continue }
                    loaded.append(AIMessage(role: .user, text: text))
                case "agentMessage":
                    if let toolMessage = toolMessageFromAgentItem(item) {
                        loaded.append(toolMessage)
                    } else if let text = visibleAssistantText(from: item), !text.isEmpty {
                        loaded.append(AIMessage(role: .assistant, text: text))
                    }
                case "dynamicToolCall":
                    loaded.append(toolMessageFromDynamicItem(item))
                default:
                    continue
                }
            }
        }
        return loaded
    }

    private func visibleUserText(from item: [String: Any]) -> String? {
        guard let content = item["content"] as? [[String: Any]] else { return nil }
        let joined = content.compactMap { $0["text"] as? String }.joined(separator: "\n")
        return cleanVisibleUserText(joined)
    }

    private func cleanVisibleUserText(_ text: String) -> String? {
        let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if trimmed.hasPrefix("Mori tool result") {
            return nil
        }
        if let range = trimmed.range(of: "User request:", options: .backwards) {
            let visible = trimmed[range.upperBound...]
                .trimmingCharacters(in: .whitespacesAndNewlines)
            return visible.isEmpty ? nil : visible
        }
        return trimmed
    }

    private func visibleAssistantText(from item: [String: Any]) -> String? {
        guard let text = item["text"] as? String else { return nil }
        if let payload = parseJSONPayload(text),
           payload["kind"] as? String == "tool" {
            return nil
        }
        return cleanAssistantText(text).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    private func toolMessageFromAgentItem(_ item: [String: Any]) -> AIMessage? {
        guard let text = item["text"] as? String,
              let payload = parseJSONPayload(text),
              payload["kind"] as? String == "tool",
              let tool = payload["tool"] as? String
        else { return nil }
        let arguments = payload["arguments"] as? [String: Any] ?? [:]
        return toolMessage(tool: tool,
                           arguments: arguments,
                           reason: payload["reason"] as? String,
                           result: nil,
                           success: nil)
    }

    private func toolMessageFromDynamicItem(_ item: [String: Any]) -> AIMessage {
        let tool = item["tool"] as? String ?? "tool"
        let arguments = item["arguments"] as? [String: Any] ?? [:]
        let result = (item["contentItems"] as? [[String: Any]])?
            .compactMap { $0["text"] as? String }
            .joined(separator: "\n")
        return toolMessage(tool: tool,
                           arguments: arguments,
                           reason: nil,
                           result: result,
                           success: item["success"] as? Bool)
    }

    private func resumeConversation(_ id: String) async throws {
        var params: [String: Any] = [
            "threadId": id,
            "approvalPolicy": "never",
            "sandbox": "danger-full-access",
            "personality": "friendly"
        ]
        if !selectedModelID.isEmpty {
            params["model"] = selectedModelID
        }
        _ = try await withTimeout(seconds: 8) {
            try await self.connection.request(method: "thread/resume", params: params)
        }
    }

    private func reasoningEfforts(from rawValue: Any?) -> [CodexReasoningEffortOption] {
        if let values = rawValue as? [[String: Any]] {
            return values.compactMap { raw in
                guard let effort = raw["reasoningEffort"] as? String else { return nil }
                return CodexReasoningEffortOption(
                    id: effort,
                    displayName: Self.displayName(forReasoningEffort: effort),
                    description: raw["description"] as? String ?? ""
                )
            }
        }
        if let values = rawValue as? [String] {
            return values.map {
                CodexReasoningEffortOption(id: $0,
                                           displayName: Self.displayName(forReasoningEffort: $0),
                                           description: "")
            }
        }
        return []
    }

    private func updateReasoningEffortsForSelectedModel(preferredEffort: String? = nil) {
        let model = modelOptions.first(where: { $0.id == selectedModelID })
        let efforts = model?.reasoningEfforts ?? []
        reasoningEffortOptions = efforts
        let preferred = preferredEffort?.isEmpty == false ? preferredEffort : model?.defaultReasoningEffort
        if let preferred, efforts.contains(where: { $0.id == preferred }) {
            selectedReasoningEffort = preferred
        } else {
            selectedReasoningEffort = efforts.first?.id ?? ""
        }
    }

    private static func displayName(forReasoningEffort effort: String) -> String {
        switch effort {
        case "xhigh": return "X High"
        default:
            return effort
                .split(separator: "-")
                .map { $0.prefix(1).uppercased() + $0.dropFirst() }
                .joined(separator: " ")
        }
    }

    private func promptForUserRequest(_ text: String) async throws -> String {
        if usesDynamicTools {
            return """
            You are Mori's built-in browser assistant. Use the Mori browser tools whenever you need page contents, tab state, or browser actions. Do not ask the user to sign in; Mori is using local Codex authentication.

            User request: \(text)
            """
        }

        guard let store else { return text }
        let snapshot = await BrowserAutomation.handle(
            tool: "mori_browser_snapshot",
            arguments: ["includePage": true, "maxTextChars": 10_000],
            store: store
        ).text
        return """
        You are Mori's built-in browser assistant. The native dynamic tool channel is unavailable in this Codex app-server version, so use this JSON protocol exactly.
        Return only one JSON object. Do not wrap it in Markdown and do not add a session summary.

        To answer, return:
        {"kind":"final","text":"..."}

        To ask Mori to perform a browser action, return:
        {"kind":"tool","tool":"mori_browser_action","arguments":{...},"reason":"..."}

        Available action arguments match this tool list:
        \(BrowserAutomation.dynamicTools)

        Current browser snapshot:
        \(snapshot)

        User request: \(text)
        """
    }

    private func handleNotification(method: String, params: [String: Any]) {
        switch method {
        case "item/agentMessage/delta":
            if let delta = findString(named: "delta", in: params), !delta.isEmpty {
                if usesDynamicTools {
                    appendToActiveAssistant(delta)
                } else {
                    pendingAssistantText += delta
                }
            }
        case "item/completed":
            if let text = completedAgentMessageText(from: params), !text.isEmpty {
                if !usesDynamicTools {
                    pendingAssistantText = text
                } else if activeAssistantText.isEmpty {
                    replaceActiveAssistantText(text)
                }
            }
        case "turn/completed":
            if let error = turnErrorMessage(from: params), activeAssistantText.isEmpty {
                replaceActiveAssistantText("Codex failed: \(error)")
                cancelTurnWatchdog()
                isWorking = false
                statusText = "Local Codex"
                return
            }
            if !usesDynamicTools {
                Task { @MainActor in await handleFallbackCompletion(params: params) }
                return
            }
            Task { @MainActor in await handleDynamicCompletion() }
        case "turn/started":
            statusText = "Working"
        case "error":
            if activeAssistantText.isEmpty {
                replaceActiveAssistantText("Codex failed: \(errorMessage(from: params))")
            }
            cancelTurnWatchdog()
            isWorking = false
            statusText = "Local Codex"
        case "thread/tokenUsage/updated":
            break
        default:
            break
        }
    }

    private func handleFallbackCompletion(params: [String: Any]) async {
        cancelTurnWatchdog()
        var raw = pendingAssistantText.isEmpty ? activeAssistantText : pendingAssistantText
        if raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           let text = await latestAssistantMessageFromThread() {
            raw = text
        }
        guard let payload = parseJSONPayload(raw),
              let kind = payload["kind"] as? String
        else {
            if raw.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                replaceActiveAssistantText("Codex finished without returning a message.")
            } else {
                replaceActiveAssistantText(cleanAssistantText(raw))
            }
            isWorking = false
            statusText = "Local Codex"
            return
        }

        if kind == "final" {
            replaceActiveAssistantText(cleanAssistantText(payload["text"] as? String ?? raw))
            isWorking = false
            statusText = "Local Codex"
            return
        }

        guard kind == "tool",
              fallbackToolIterations < 8,
              let threadId,
              let store,
              let tool = payload["tool"] as? String
        else {
            replaceActiveAssistantText("I could not complete the browser action.")
            isWorking = false
            statusText = "Local Codex"
            return
        }

        fallbackToolIterations += 1
        let arguments = payload["arguments"] as? [String: Any] ?? [:]
        pendingAssistantText = ""
        let toolMessageId = beginToolCall(tool: tool,
                                          arguments: arguments,
                                          reason: payload["reason"] as? String)
        statusText = "Using \(tool.replacingOccurrences(of: "mori_", with: ""))"
        let result = await BrowserAutomation.handle(tool: tool, arguments: arguments, store: store)
        finishToolCall(toolMessageId, result: result.text, success: result.success)
        let prompt = """
        Mori tool result for \(tool), success=\(result.success):
        \(result.text)

        Continue the same JSON protocol. Return only one JSON object: either the next tool call or {"kind":"final","text":"..."}. Do not add a session summary.
        """
        do {
            try await startTurn(threadId: threadId, prompt: prompt)
        } catch {
            replaceActiveAssistantText("The browser tool ran, but Codex could not continue: \(error.localizedDescription)")
            isWorking = false
            statusText = "Local Codex"
        }
    }

    private func handleDynamicCompletion() async {
        cancelTurnWatchdog()
        if activeAssistantText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty,
           let text = await latestAssistantMessageFromThread(),
           !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            replaceActiveAssistantText(cleanAssistantText(text))
        }
        if activeAssistantText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            replaceActiveAssistantText("Done.")
        }
        isWorking = false
        statusText = "Local Codex"
    }

    private func armTurnWatchdog() {
        turnWatchdogTask?.cancel()
        turnWatchdogTask = Task { [weak self] in
            do {
                try await Task.sleep(nanoseconds: 90_000_000_000)
            } catch {
                return
            }
            await self?.handleTurnTimeout()
        }
    }

    private func cancelTurnWatchdog() {
        turnWatchdogTask?.cancel()
        turnWatchdogTask = nil
    }

    private func handleTurnTimeout() async {
        guard isWorking else { return }
        statusText = "Reading Codex"
        if let text = await latestAssistantMessageFromThread(),
           !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            replaceActiveAssistantText(cleanAssistantText(text))
        } else if activeAssistantText.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            replaceActiveAssistantText("Codex is taking longer than expected. Please try again.")
        }
        isWorking = false
        statusText = "Local Codex"
    }

    private func latestAssistantMessageFromThread() async -> String? {
        guard let threadId else { return nil }
        do {
            let response = try await withTimeout(seconds: 8) {
                try await self.connection.request(
                    method: "thread/read",
                    params: ["threadId": threadId, "includeTurns": true]
                )
            }
            guard let thread = response["thread"] as? [String: Any],
                  let turns = thread["turns"] as? [[String: Any]]
            else { return nil }
            for turn in turns.reversed() {
                guard let items = turn["items"] as? [[String: Any]] else { continue }
                for item in items.reversed() {
                    if item["type"] as? String == "agentMessage",
                       let text = item["text"] as? String,
                       !text.isEmpty {
                        return text
                    }
                }
            }
        } catch {
            return nil
        }
        return nil
    }

    private func handleServerRequest(method: String, params: [String: Any]) async -> [String: Any] {
        guard method == "item/tool/call" else {
            return [
                "contentItems": [
                    ["type": "inputText", "text": "Unsupported app-server request: \(method)"]
                ],
                "success": false
            ]
        }
        guard let store else {
            return [
                "contentItems": [
                    ["type": "inputText", "text": "The browser store is unavailable."]
                ],
                "success": false
            ]
        }
        let tool = params["tool"] as? String ?? ""
        let arguments = params["arguments"] as? [String: Any] ?? [:]
        let toolMessageId = beginToolCall(tool: tool, arguments: arguments, reason: nil)
        statusText = "Using \(tool.replacingOccurrences(of: "mori_", with: ""))"
        let result = await BrowserAutomation.handle(tool: tool, arguments: arguments, store: store)
        finishToolCall(toolMessageId, result: result.text, success: result.success)
        statusText = "Working"
        return result.rpcResult
    }

    private var activeAssistantText: String {
        guard let activeAssistantMessageId,
              let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId })
        else { return "" }
        return messages[index].text
    }

    private func appendToActiveAssistant(_ text: String) {
        guard let activeAssistantMessageId,
              let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId })
        else { return }
        messages[index].text += text
    }

    private func replaceActiveAssistantText(_ text: String) {
        guard let activeAssistantMessageId,
              let index = messages.firstIndex(where: { $0.id == activeAssistantMessageId })
        else { return }
        messages[index].text = text
    }

    private func beginToolCall(tool: String,
                               arguments: [String: Any],
                               reason: String?) -> AIMessage.ID {
        let message = toolMessage(tool: tool,
                                  arguments: arguments,
                                  reason: reason,
                                  result: nil,
                                  success: nil)
        if let index = activeAssistantIndex(),
           messages[index].role == .assistant,
           messages[index].text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            messages[index] = message
        } else {
            messages.append(message)
        }
        let placeholder = AIMessage(role: .assistant, text: "")
        messages.append(placeholder)
        activeAssistantMessageId = placeholder.id
        return message.id
    }

    private func finishToolCall(_ id: AIMessage.ID,
                                result: String,
                                success: Bool) {
        guard let index = messages.firstIndex(where: { $0.id == id }),
              var toolCall = messages[index].toolCall
        else { return }
        toolCall.result = clipped(result, maxLength: 4_000)
        toolCall.success = success
        messages[index].toolCall = toolCall
    }

    private func activeAssistantIndex() -> Int? {
        guard let activeAssistantMessageId else { return nil }
        return messages.firstIndex(where: { $0.id == activeAssistantMessageId })
    }

    private func toolMessage(tool: String,
                             arguments: [String: Any],
                             reason: String?,
                             result: String?,
                             success: Bool?) -> AIMessage {
        let info = AIToolCallInfo(
            title: toolTitle(tool: tool, arguments: arguments),
            name: tool,
            arguments: prettyJSON(arguments),
            reason: reason,
            result: result.map { clipped($0, maxLength: 4_000) },
            success: success
        )
        return AIMessage(role: .tool, text: info.title, toolCall: info)
    }

    private func toolTitle(tool: String, arguments: [String: Any]) -> String {
        if tool == "mori_browser_action",
           let action = arguments["action"] as? String {
            return "Browser \(humanizedToolName(action))"
        }
        if tool == "mori_browser_snapshot" {
            return "Read browser"
        }
        return humanizedToolName(tool
            .replacingOccurrences(of: "mori_", with: "")
            .replacingOccurrences(of: "browser_", with: ""))
    }

    private func humanizedToolName(_ raw: String) -> String {
        let spaced = raw
            .replacingOccurrences(of: "([a-z])([A-Z])",
                                  with: "$1 $2",
                                  options: .regularExpression)
            .replacingOccurrences(of: "_", with: " ")
            .replacingOccurrences(of: "-", with: " ")
        return spaced
            .split(separator: " ")
            .map { $0.prefix(1).uppercased() + $0.dropFirst() }
            .joined(separator: " ")
    }

    private func prettyJSON(_ value: Any) -> String {
        guard JSONSerialization.isValidJSONObject(value),
              let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]),
              let text = String(data: data, encoding: .utf8)
        else { return String(describing: value) }
        return text
    }

    private func clipped(_ text: String, maxLength: Int) -> String {
        guard text.count > maxLength else { return text }
        return String(text.prefix(maxLength)) + "\n..."
    }

    private func findString(named name: String, in value: Any) -> String? {
        if let dict = value as? [String: Any] {
            if let direct = dict[name] as? String { return direct }
            for child in dict.values {
                if let match = findString(named: name, in: child) { return match }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let match = findString(named: name, in: child) { return match }
            }
        }
        return nil
    }

    private func findFirstText(in value: Any) -> String? {
        if let dict = value as? [String: Any] {
            if let text = dict["text"] as? String { return text }
            if let content = dict["content"] as? String { return content }
            for child in dict.values {
                if let match = findFirstText(in: child) { return match }
            }
        } else if let array = value as? [Any] {
            for child in array {
                if let match = findFirstText(in: child) { return match }
            }
        }
        return nil
    }

    private func completedAgentMessageText(from params: [String: Any]) -> String? {
        guard let item = params["item"] as? [String: Any],
              item["type"] as? String == "agentMessage",
              let text = item["text"] as? String
        else { return nil }
        return text
    }

    private func cleanAssistantText(_ text: String) -> String {
        if let payload = parseJSONPayload(text),
           payload["kind"] as? String == "final",
           let finalText = payload["text"] as? String {
            return stripSessionSummary(finalText)
        }
        return stripSessionSummary(text)
    }

    private func stripSessionSummary(_ text: String) -> String {
        guard let range = text.range(of: "**Session Summary**") else {
            return text
        }
        var prefix = String(text[..<range.lowerBound])
            .trimmingCharacters(in: .whitespacesAndNewlines)
        if let lastLine = prefix.split(separator: "\n", omittingEmptySubsequences: false).last,
           lastLine.unicodeScalars.allSatisfy({ !CharacterSet.alphanumerics.contains($0) }) {
            prefix = prefix.split(separator: "\n", omittingEmptySubsequences: false)
                .dropLast()
                .joined(separator: "\n")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        return prefix
    }

    private func parseJSONPayload(_ raw: String) -> [String: Any]? {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        if text.hasPrefix("```") {
            text = text.replacingOccurrences(of: "```json", with: "")
                .replacingOccurrences(of: "```", with: "")
                .trimmingCharacters(in: .whitespacesAndNewlines)
        }
        if let object = decodeJSONObject(text) {
            return object
        }
        guard let objectText = firstJSONObject(in: text),
              let object = decodeJSONObject(objectText)
        else {
            return nil
        }
        return object
    }

    private func decodeJSONObject(_ text: String) -> [String: Any]? {
        guard let data = text.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }

    private func firstJSONObject(in text: String) -> String? {
        guard let start = text.firstIndex(of: "{") else { return nil }
        var depth = 0
        var isInString = false
        var isEscaped = false
        var index = start
        while index < text.endIndex {
            let character = text[index]
            if isInString {
                if isEscaped {
                    isEscaped = false
                } else if character == "\\" {
                    isEscaped = true
                } else if character == "\"" {
                    isInString = false
                }
            } else if character == "\"" {
                isInString = true
            } else if character == "{" {
                depth += 1
            } else if character == "}" {
                depth -= 1
                if depth == 0 {
                    return String(text[start...index])
                }
            }
            index = text.index(after: index)
        }
        return nil
    }

    private func number(_ value: Any?) -> Double? {
        if let double = value as? Double { return double }
        if let int = value as? Int { return Double(int) }
        if let number = value as? NSNumber { return number.doubleValue }
        if let string = value as? String { return Double(string) }
        return nil
    }

    private func turnErrorMessage(from params: [String: Any]) -> String? {
        guard let turn = params["turn"] as? [String: Any],
              turn["status"] as? String == "failed"
        else { return nil }
        if let error = turn["error"] as? [String: Any] {
            return errorMessage(from: ["error": error])
        }
        return "The turn failed before Codex returned an answer."
    }

    private func errorMessage(from params: [String: Any]) -> String {
        let error = params["error"] as? [String: Any]
        let raw = error?["message"] as? String ?? "Unknown app-server error."
        guard let data = raw.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let nested = object["error"] as? [String: Any],
              let message = nested["message"] as? String
        else { return raw }
        return message
    }

    private func withTimeout<T>(seconds: Double, operation: @escaping () async throws -> T) async throws -> T {
        try await withThrowingTaskGroup(of: T.self) { group in
            group.addTask { try await operation() }
            group.addTask {
                try await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
                throw CodexAppServerError.protocolError("Timed out waiting for Codex app server.")
            }
            guard let result = try await group.next() else {
                throw CodexAppServerError.connectionFailed
            }
            group.cancelAll()
            return result
        }
    }
}

enum CodexAppServerError: LocalizedError {
    case codexBinaryMissing
    case connectionFailed
    case serverError(String)
    case protocolError(String)

    var errorDescription: String? {
        switch self {
        case .codexBinaryMissing:
            return "Could not find the codex CLI. Install Codex or set CODEX_BIN."
        case .connectionFailed:
            return "Could not connect to the local Codex app server."
        case .serverError(let message), .protocolError(let message):
            return message
        }
    }
}

@MainActor
final class CodexAppServerConnection {
    var onNotification: ((String, [String: Any]) -> Void)?
    var onServerRequest: ((String, [String: Any]) async -> [String: Any])?

    private var process: Process?
    private var socket: URLSessionWebSocketTask?
    private var receiveTask: Task<Void, Never>?
    private var continuations: [Int: CheckedContinuation<[String: Any], Error>] = [:]
    private var nextId = 1
    private let port = Int.random(in: 42_200...42_999)
    private var initialized = false

    deinit {
        socket?.cancel(with: .goingAway, reason: nil)
        process?.terminate()
    }

    func connectIfNeeded() async throws {
        if socket != nil { return }
        try launchServerIfNeeded()
        let url = URL(string: "ws://127.0.0.1:\(port)")!
        var lastError: Error?
        for _ in 0..<20 {
            let task = URLSession.shared.webSocketTask(with: url)
            task.resume()
            socket = task
            receiveTask = Task { [weak self] in await self?.receiveLoop() }
            do {
                try await initialize()
                _ = try await request(method: "model/list", params: [:])
                return
            } catch {
                lastError = error
                socket?.cancel(with: .goingAway, reason: nil)
                socket = nil
                receiveTask?.cancel()
                try await Task.sleep(nanoseconds: 150_000_000)
            }
        }
        throw lastError ?? CodexAppServerError.connectionFailed
    }

    func request(method: String, params: [String: Any]) async throws -> [String: Any] {
        guard let socket else { throw CodexAppServerError.connectionFailed }
        let id = nextId
        nextId += 1
        let payload: [String: Any] = ["id": id, "method": method, "params": params]
        return try await withCheckedThrowingContinuation { continuation in
            continuations[id] = continuation
            Task { @MainActor in
                do {
                    try await send(payload, on: socket)
                } catch {
                    if let pending = continuations.removeValue(forKey: id) {
                        pending.resume(throwing: error)
                    }
                }
            }
        }
    }

    private func initialize() async throws {
        guard !initialized else { return }
        _ = try await request(
            method: "initialize",
            params: [
                "clientInfo": ["name": "Mori", "version": "0"],
                "capabilities": [
                    "experimentalApi": true,
                    "requestAttestation": false
                ]
            ]
        )
        initialized = true
    }

    private func launchServerIfNeeded() throws {
        if let process, process.isRunning { return }
        let codex = try codexBinaryPath()
        let process = Process()
        process.executableURL = URL(fileURLWithPath: codex)
        process.arguments = ["app-server", "--listen", "ws://127.0.0.1:\(port)"]
        process.environment = ProcessInfo.processInfo.environment
        let output = Pipe()
        output.fileHandleForReading.readabilityHandler = { handle in
            _ = handle.availableData
        }
        process.standardOutput = output
        process.standardError = output
        try process.run()
        self.process = process
    }

    private func codexBinaryPath() throws -> String {
        let env = ProcessInfo.processInfo.environment
        let candidates = [
            env["CODEX_BIN"],
            "\(NSHomeDirectory())/.bun/bin/codex",
            "\(NSHomeDirectory())/.npm-global/bin/codex",
            "/opt/homebrew/bin/codex",
            "/usr/local/bin/codex"
        ].compactMap { $0 }

        for path in candidates where FileManager.default.isExecutableFile(atPath: path) {
            return path
        }
        throw CodexAppServerError.codexBinaryMissing
    }

    private func receiveLoop() async {
        while !Task.isCancelled, let socket {
            do {
                let message = try await socket.receive()
                try await handle(message)
            } catch {
                failPending(error)
                break
            }
        }
    }

    private func handle(_ message: URLSessionWebSocketTask.Message) async throws {
        let data: Data
        switch message {
        case .data(let incoming):
            data = incoming
        case .string(let string):
            data = Data(string.utf8)
        @unknown default:
            return
        }
        guard let object = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            return
        }

        let id = object["id"] as? Int
        let method = object["method"] as? String
        let params = object["params"] as? [String: Any] ?? [:]

        if let method, let id {
            let result = await onServerRequest?(method, params) ?? [:]
            try await send(["id": id, "result": result], on: socket!)
            return
        }

        if let method {
            onNotification?(method, params)
            return
        }

        if let id, let continuation = continuations.removeValue(forKey: id) {
            if let error = object["error"] as? [String: Any] {
                let message = error["message"] as? String ?? "Codex app-server request failed."
                continuation.resume(throwing: CodexAppServerError.serverError(message))
                return
            }
            let result = object["result"] as? [String: Any] ?? [:]
            continuation.resume(returning: result)
        }
    }

    private func send(_ payload: [String: Any], on socket: URLSessionWebSocketTask) async throws {
        guard JSONSerialization.isValidJSONObject(payload) else {
            throw CodexAppServerError.protocolError("Attempted to send invalid JSON-RPC payload.")
        }
        let data = try JSONSerialization.data(withJSONObject: payload)
        guard let text = String(data: data, encoding: .utf8) else {
            throw CodexAppServerError.protocolError("Could not encode JSON-RPC payload.")
        }
        try await socket.send(.string(text))
    }

    private func failPending(_ error: Error) {
        let pending = continuations
        continuations.removeAll()
        pending.values.forEach { $0.resume(throwing: error) }
    }
}

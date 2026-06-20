import SwiftUI
import AppKit

/// A single transient notification surfaced in the corner of the window.
///
/// Toasts are intentionally tiny and self-describing: a message, an optional
/// leading icon, and a semantic `style` that drives the accent color. New kinds
/// of notification should reuse this type rather than inventing bespoke banners,
/// so everything that pops up shares one look, placement, and timing.
struct Toast: Identifiable, Equatable {
    enum Style {
        case info, success, warning, error
    }

    let id = UUID()
    var message: String
    /// Icon identifier (SF Symbol or Nucleo name); resolved by `Icon`.
    var icon: String?
    var style: Style = .info
    /// Seconds the toast lingers before auto-dismissing.
    var duration: TimeInterval = 2.4

    static func == (lhs: Toast, rhs: Toast) -> Bool { lhs.id == rhs.id }
}

/// App-wide hub for transient toast notifications.
///
/// Any part of the browser — store actions, extension handlers, keyboard
/// commands — can call `show(...)` to flash a brief, self-dismissing message;
/// `ToastOverlay` renders whatever is queued here. Centralizing the queue keeps
/// notifications consistent and stops features from stacking ad-hoc UI.
///
/// Main-thread only: callers and the auto-dismiss timer all run on the main
/// queue, matching how the rest of the chrome mutates `@Published` state.
final class ToastCenter: ObservableObject {
    static let shared = ToastCenter()

    /// Currently visible toasts, oldest first.
    @Published private(set) var toasts: [Toast] = []

    /// Cap on simultaneously visible toasts; older ones fall off the top.
    private let maxVisible = 3
    private var dismissWorkItems: [Toast.ID: DispatchWorkItem] = [:]

    init() {}

    /// Queue a notification. Returns its id so a caller can dismiss it early.
    @discardableResult
    func show(_ message: String,
              icon: String? = nil,
              style: Toast.Style = .info,
              duration: TimeInterval = 2.4) -> Toast.ID {
        let toast = Toast(message: message, icon: icon, style: style, duration: duration)
        withAnimation(Motion.snappy) {
            toasts.append(toast)
            while toasts.count > maxVisible {
                let dropped = toasts.removeFirst()
                cancelDismiss(for: dropped.id)
            }
        }
        scheduleDismiss(toast)
        return toast.id
    }

    /// Dismiss a toast now (e.g. on tap), cancelling its pending timer.
    func dismiss(_ id: Toast.ID) {
        cancelDismiss(for: id)
        withAnimation(Motion.snappy) {
            toasts.removeAll { $0.id == id }
        }
    }

    private func scheduleDismiss(_ toast: Toast) {
        let work = DispatchWorkItem { [weak self] in
            self?.dismiss(toast.id)
        }
        dismissWorkItems[toast.id] = work
        DispatchQueue.main.asyncAfter(deadline: .now() + toast.duration, execute: work)
    }

    private func cancelDismiss(for id: Toast.ID) {
        dismissWorkItems[id]?.cancel()
        dismissWorkItems.removeValue(forKey: id)
    }
}

struct PermissionPromptItem: Identifiable, Equatable {
    enum Response: Int {
        case allow = 0
        case block = 1
        case dismiss = 2
    }

    let id: Int
    let origin: String
    let requests: [String]
    let completion: (Response) -> Void

    static func == (lhs: PermissionPromptItem, rhs: PermissionPromptItem) -> Bool {
        lhs.id == rhs.id
    }
}

final class PermissionPromptCenter: ObservableObject {
    static let shared = PermissionPromptCenter()

    @Published private(set) var prompts: [PermissionPromptItem] = []

    private init() {}

    func show(id: Int,
              origin: String,
              requests: [String],
              completion: @escaping (PermissionPromptItem.Response) -> Void) {
        let prompt = PermissionPromptItem(id: id,
                                          origin: origin,
                                          requests: requests,
                                          completion: completion)
        withAnimation(Motion.snappy) {
            prompts.removeAll { $0.id == id }
            prompts.append(prompt)
        }
    }

    func dismiss(id: Int) {
        withAnimation(Motion.snappy) {
            prompts.removeAll { $0.id == id }
        }
    }

    func respond(to prompt: PermissionPromptItem,
                 with response: PermissionPromptItem.Response) {
        dismiss(id: prompt.id)
        prompt.completion(response)
    }
}

/// ObjC-callable bridge for Chromium's permission prompt controller.
@objc(MoriPermissionBridge)
final class MoriPermissionBridge: NSObject {
    @objc(showPermissionPromptWithID:origin:requests:completion:)
    static func showPermissionPrompt(id: Int,
                                     origin: String,
                                     requests: [String],
                                     completion: @escaping (Int) -> Void) {
        DispatchQueue.main.async {
            PermissionPromptCenter.shared.show(id: id,
                                               origin: origin,
                                               requests: requests) { response in
                completion(response.rawValue)
            }
        }
    }

    @objc(dismissPermissionPromptWithID:)
    static func dismissPermissionPrompt(id: Int) {
        DispatchQueue.main.async {
            PermissionPromptCenter.shared.dismiss(id: id)
        }
    }
}

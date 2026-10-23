#include "chrome/browser/ui/views/mori/mori_assistant.h"

#include <algorithm>
#include <map>
#include <limits>
#include <memory>
#include <optional>
#include <set>
#include <string>
#include <utility>
#include <vector>

#include "base/functional/bind.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/weak_ptr.h"
#include "base/strings/string_util.h"
#include "base/strings/utf_string_conversions.h"
#include "base/task/single_thread_task_runner.h"
#include "base/timer/timer.h"
#include "cc/paint/paint_flags.h"
#include "chrome/browser/profiles/profile.h"
#include "chrome/browser/ui/browser_window/public/browser_window_interface.h"
#include "chrome/browser/ui/browser_tabstrip.h"
#include "chrome/browser/ui/color/chrome_color_id.h"
#include "chrome/browser/ui/tabs/tab_strip_model.h"
#include "chrome/browser/ui/views/frame/browser_view.h"
#include "chrome/browser/ui/views/mori/mori_assistant_tools.h"
#include "chrome/browser/ui/views/mori/mori_codex.h"
#include "chrome/browser/ui/views/mori/mori_icons.h"
#include "chrome/browser/ui/views/mori/mori_markdown.h"
#include "content/public/browser/web_contents.h"
#include "ui/base/class_property.h"
#include "ui/base/metadata/metadata_header_macros.h"
#include "ui/base/metadata/metadata_impl_macros.h"
#include "ui/base/models/image_model.h"
#include "ui/color/color_provider.h"
#include "ui/events/keycodes/keyboard_codes.h"
#include "ui/gfx/canvas.h"
#include "ui/gfx/text_utils.h"
#include "ui/gfx/color_utils.h"
#include "ui/gfx/font_list.h"
#include "ui/gfx/geometry/rect_f.h"
#include "ui/gfx/shadow_value.h"
#include "ui/gfx/skia_paint_util.h"
#include "ui/views/accessibility/view_accessibility.h"
#include "ui/views/background.h"
#include "ui/views/border.h"
#include "ui/views/controls/button/image_button.h"
#include "ui/views/controls/button/image_button_factory.h"
#include "ui/views/controls/button/button.h"
#include "ui/views/controls/image_view.h"
#include "ui/views/controls/label.h"
#include "ui/views/controls/link.h"
#include "ui/views/controls/styled_label.h"
#include "ui/views/controls/scroll_view.h"
#include "ui/views/controls/separator.h"
#include "ui/views/controls/textfield/textfield.h"
#include "ui/views/controls/textfield/textfield_controller.h"
#include "ui/views/layout/box_layout.h"
#include "ui/views/layout/box_layout_view.h"
#include "ui/views/view.h"
#include "ui/views/view_class_properties.h"
#include "ui/views/view_utils.h"

namespace mori {
namespace {

constexpr char kApprovalPolicy[] = "on-request";
constexpr char kSandbox[] = "workspace-write";
constexpr char kServiceName[] = "mori_browser";
// A turn with no server activity this long is considered stalled; the
// clock stops while an approval card is up.
constexpr base::TimeDelta kTurnIdleTimeout = base::Seconds(90);
constexpr base::TimeDelta kRequestTimeout = base::Seconds(8);
constexpr base::TimeDelta kTurnStartTimeout = base::Seconds(15);
constexpr base::TimeDelta kThreadStartTimeout = base::Seconds(20);
constexpr char kDisconnected[] = "The local Codex app server disconnected.";

// ---- Protocol helpers -----------------------------------------------------

std::optional<std::string> ThreadIdOf(const base::DictValue& params) {
  if (const std::string* id = params.FindString("threadId")) {
    return *id;
  }
  if (const std::string* id = params.FindStringByDottedPath("thread.id")) {
    return *id;
  }
  return std::nullopt;
}

std::optional<std::string> TurnIdOf(const base::DictValue& params) {
  if (const std::string* id = params.FindString("turnId")) {
    return *id;
  }
  if (const std::string* id = params.FindStringByDottedPath("turn.id")) {
    return *id;
  }
  return std::nullopt;
}

std::string ErrorMessageOf(const base::DictValue& params) {
  if (const std::string* message =
          params.FindStringByDottedPath("error.message")) {
    return *message;
  }
  if (const std::string* message = params.FindString("message")) {
    return *message;
  }
  return "Unknown error.";
}

// ---- Session ----------------------------------------------------------------

enum class Role { kUser, kAssistant, kTool, kError };

struct Message {
  Role role;
  std::string text;
};

class AssistantSession : public CodexServer::Delegate {
 public:
  AssistantSession(BrowserWindowInterface* browser,
                   base::RepeatingClosure changed)
      : browser_(browser->GetWeakPtr()), changed_(std::move(changed)) {}

  ~AssistantSession() override { DenyApproval(); }

  const std::vector<Message>& messages() const { return messages_; }
  bool busy() const { return busy_; }
  const std::string& status() const { return status_; }
  const std::optional<std::string>& approval() const { return approval_; }

  void Send(std::string text) {
    text = std::string(base::TrimWhitespaceASCII(text, base::TRIM_ALL));
    if (text.empty() || busy_ || !browser_) {
      return;
    }
    messages_.push_back({Role::kUser, text});
    pending_text_ = text;
    ++generation_;
    stop_requested_ = false;
    active_turn_.reset();
    BeginTurnOutput();
    busy_ = true;
    status_ = "Starting Codex…";
    Bump();
    Changed();
    EnsureConnected(generation_);
  }

  // A new conversation (a new Codex thread with the next message).
  void Reset() {
    if (busy_) {
      StopActiveTurn();
    }
    ++generation_;
    DenyApproval();
    messages_.clear();
    thread_id_.reset();
    stop_requested_ = false;
    BeginTurnOutput();
    EndTurn();
    Changed();
  }

  void Interrupt() {
    if (!busy_) {
      return;
    }
    stop_requested_ = true;
    ++generation_;
    StopActiveTurn();
    DenyApproval();
    if (!turn_has_text_) {
      messages_.push_back({Role::kAssistant, "Stopped."});
    }
    EndTurn();
    Changed();
  }

  void ResolveApproval(bool allow) {
    if (!approval_reply_) {
      return;
    }
    approval_.reset();
    base::OnceCallback<void(bool)> reply = std::move(approval_reply_);
    Bump();
    if (busy_) {
      status_ = allow ? "Working…" : "Thinking…";
    }
    Changed();
    std::move(reply).Run(allow);
  }

  // CodexServer::Delegate:
  void OnCodexNotification(const std::string& method,
                           const base::DictValue& params) override {
    if (std::optional<std::string> theirs = ThreadIdOf(params);
        theirs && thread_id_ && *theirs != *thread_id_) {
      return;
    }
    std::optional<std::string> turn = TurnIdOf(params);
    if (ShouldIgnore(method, turn) || !busy_) {
      return;
    }
    // Any notification, reasoning deltas included, is a sign of life.
    Bump();
    if (method == "item/agentMessage/delta") {
      const std::string* delta = params.FindString("delta");
      if (delta && !delta->empty()) {
        const std::string* item = params.FindString("itemId");
        Bubble(item ? std::optional<std::string>(*item) : std::nullopt)
            .text += *delta;
        turn_has_text_ = true;
      }
    } else if (method == "item/completed") {
      const base::DictValue* item = params.FindDict("item");
      const std::string* type = item ? item->FindString("type") : nullptr;
      const std::string* text = item ? item->FindString("text") : nullptr;
      if (type && *type == "agentMessage" && text &&
          !base::TrimWhitespaceASCII(*text, base::TRIM_ALL).empty()) {
        const std::string* id = item->FindString("id");
        Bubble(id ? std::optional<std::string>(*id) : std::nullopt).text =
            *text;
        turn_has_text_ = true;
      }
    } else if (method == "turn/started") {
      if (turn) {
        active_turn_ = turn;
      }
      if (!approval_) {
        status_ = "Working…";
      }
    } else if (method == "turn/completed") {
      if (const std::string* error =
              params.FindStringByDottedPath("turn.error.message")) {
        FailTurn("Codex failed: " + *error);
        return;
      }
      if (!turn_has_text_) {
        messages_.push_back({Role::kAssistant, "Done."});
      }
      EndTurn();
    } else if (method == "error") {
      const std::string message = ErrorMessageOf(params);
      if (params.FindBool("willRetry").value_or(false)) {
        status_ = "Retrying… (" + Clip(message, 80) + ")";
      } else {
        FailTurn("Codex failed: " + message);
        return;
      }
    } else {
      return;
    }
    Changed();
  }

  void OnCodexRequest(base::Value id,
                      const std::string& method,
                      base::DictValue params) override {
    if (!server_) {
      return;
    }
    Bump();
    if (method == "item/tool/call") {
      StartToolCall(std::move(id), std::move(params));
    } else if (method == "item/commandExecution/requestApproval" ||
               method == "item/fileChange/requestApproval") {
      // A browser agent: no shell or file access beyond its sandbox.
      server_->Respond(id, base::Value(base::DictValue().Set("decision",
                                                            "decline")));
      NoteDeclined(method);
    } else if (method == "execCommandApproval" ||
               method == "applyPatchApproval") {
      server_->Respond(id, base::Value(base::DictValue().Set("decision",
                                                            "denied")));
      NoteDeclined(method);
    } else if (method == "mcpServer/elicitation/request") {
      server_->Respond(id, base::Value(base::DictValue().Set("action",
                                                            "decline")));
    } else {
      server_->RespondError(id, -32601, "Mori does not support " + method + ".");
    }
  }

  void OnCodexClosed(const std::string& reason) override {
    // Not from inside the server's own call.
    base::SingleThreadTaskRunner::GetCurrentDefault()->DeleteSoon(
        FROM_HERE, std::move(server_));
    initialized_ = false;
    thread_id_.reset();
    if (busy_) {
      ++generation_;
      FailTurn(reason + " Send your message again to restart it.");
    }
  }

 private:
  void Changed() { changed_.Run(); }

  void Bump() { last_activity_ = base::TimeTicks::Now(); }

  // Where the app server runs (and the only place its sandbox may write).
  base::FilePath WorkingDir() {
    return browser_->GetProfile()->GetPath().AppendASCII("Mori Assistant");
  }

  void EnsureConnected(uint64_t generation) {
    if (server_ && initialized_) {
      EnsureThread(generation);
      return;
    }
    if (!server_) {
      server_ = std::make_unique<CodexServer>(WorkingDir(), this);
    }
    status_ = "Connecting to Codex…";
    Changed();
    server_->Request(
        "initialize",
        base::DictValue()
            .Set("clientInfo",
                 base::DictValue().Set("name", "Mori").Set("version", "3"))
            .Set("capabilities", base::DictValue()
                                     .Set("experimentalApi", true)
                                     .Set("requestAttestation", false)),
        kRequestTimeout,
        base::BindOnce(&AssistantSession::OnInitialized,
                       weak_factory_.GetWeakPtr(), generation));
  }

  void OnInitialized(uint64_t generation,
                     std::optional<base::Value> result,
                     std::string error) {
    if (!result) {
      Fail(generation, "Couldn't initialize the Codex app server: " + error);
      return;
    }
    if (!server_) {
      return;
    }
    server_->Notify("initialized", base::DictValue());
    initialized_ = true;
    thread_id_.reset();
    if (generation == generation_) {
      EnsureThread(generation);
    }
  }

  void EnsureThread(uint64_t generation) {
    if (thread_id_) {
      StartTurn(generation);
      return;
    }
    base::DictValue params =
        base::DictValue()
            .Set("cwd", WorkingDir().value())
            .Set("approvalPolicy", kApprovalPolicy)
            .Set("sandbox", kSandbox)
            .Set("personality", "friendly")
            .Set("serviceName", kServiceName);
    const bool tools = !tools_unsupported_;
    if (tools) {
      params.Set("dynamicTools", DynamicTools());
    }
    server_->Request("thread/start", std::move(params), kThreadStartTimeout,
                     base::BindOnce(&AssistantSession::OnThreadStarted,
                                    weak_factory_.GetWeakPtr(), generation,
                                    tools));
  }

  void OnThreadStarted(uint64_t generation,
                       bool tools,
                       std::optional<base::Value> result,
                       std::string error) {
    if (!result) {
      // An older Codex without dynamic tools: chat without the browser.
      if (tools && base::ToLowerASCII(error).find("dynamictools") !=
                       std::string::npos) {
        tools_unsupported_ = true;
        messages_.push_back(
            {Role::kTool,
             "This Codex version doesn't support Mori's browser tools, so "
             "the assistant can't see or use your tabs. Update Codex to "
             "enable them."});
        Changed();
        if (server_) {
          EnsureThread(generation);
        }
        return;
      }
      Fail(generation, error);
      return;
    }
    const std::string* id =
        result->is_dict() ? result->GetDict().FindStringByDottedPath("thread.id")
                          : nullptr;
    if (!id || id->empty()) {
      Fail(generation, "thread/start did not return a thread id.");
      return;
    }
    // Kept even if the user stopped meanwhile; the next message reuses it.
    thread_id_ = *id;
    uses_tools_ = tools;
    if (generation == generation_) {
      StartTurn(generation);
    }
  }

  void StartTurn(uint64_t generation) {
    status_ = "Thinking…";
    Changed();
    server_->Request(
        "turn/start",
        base::DictValue()
            .Set("threadId", *thread_id_)
            .Set("input",
                 base::ListValue().Append(
                     base::DictValue()
                         .Set("type", "text")
                         .Set("text", PromptFor(pending_text_, uses_tools_))))
            .Set("approvalPolicy", kApprovalPolicy),
        kTurnStartTimeout,
        base::BindOnce(&AssistantSession::OnTurnStarted,
                       weak_factory_.GetWeakPtr(), generation));
  }

  void OnTurnStarted(uint64_t generation,
                     std::optional<base::Value> result,
                     std::string error) {
    if (!result) {
      Fail(generation, error);
      return;
    }
    std::optional<std::string> turn =
        result->is_dict() ? TurnIdOf(result->GetDict()) : std::nullopt;
    if (generation != generation_) {
      // Stopped while turn/start was in flight: stop that turn too.
      if (turn) {
        ignored_turns_.insert(*turn);
        InterruptRemote(*turn);
      }
      return;
    }
    if (!active_turn_) {
      active_turn_ = turn;
    }
    Bump();
    watchdog_.Start(FROM_HERE, base::Seconds(5),
                    base::BindRepeating(&AssistantSession::CheckIdle,
                                        base::Unretained(this)));
  }

  void Fail(uint64_t generation, const std::string& message) {
    if (generation == generation_ && busy_) {
      FailTurn(message);
    }
  }

  void FailTurn(const std::string& message) {
    messages_.push_back({Role::kError, message});
    EndTurn();
    Changed();
  }

  void BeginTurnOutput() {
    agent_items_.clear();
    loose_assistant_.reset();
    turn_has_text_ = false;
  }

  void EndTurn() {
    busy_ = false;
    status_.clear();
    active_turn_.reset();
    watchdog_.Stop();
    DenyApproval();
  }

  void StopActiveTurn() {
    if (active_turn_) {
      ignored_turns_.insert(*active_turn_);
      InterruptRemote(*active_turn_);
      active_turn_.reset();
    }
  }

  void InterruptRemote(const std::string& turn) {
    if (server_ && thread_id_) {
      server_->Request(
          "turn/interrupt",
          base::DictValue().Set("threadId", *thread_id_).Set("turnId", turn),
          kRequestTimeout, base::DoNothing());
    }
  }

  void CheckIdle() {
    if (!busy_) {
      watchdog_.Stop();
      return;
    }
    if (approval_) {
      Bump();
      return;
    }
    if (base::TimeTicks::Now() - last_activity_ < kTurnIdleTimeout) {
      return;
    }
    StopActiveTurn();
    ++generation_;
    FailTurn(
        "Codex stopped responding for 90 seconds, so this request was "
        "cancelled. Please try again.");
  }

  // Notifications of stopped turns are dropped.
  bool ShouldIgnore(const std::string& method,
                    const std::optional<std::string>& turn) {
    if (turn && ignored_turns_.contains(*turn)) {
      if (method == "turn/completed" || method == "error") {
        ignored_turns_.erase(*turn);
      }
      return true;
    }
    if (!stop_requested_) {
      return false;
    }
    if (method == "turn/started" && turn) {
      // A turn that started after the user stopped: stop it too.
      ignored_turns_.insert(*turn);
      InterruptRemote(*turn);
    }
    return true;
  }

  Message& Bubble(std::optional<std::string> item) {
    std::optional<size_t> existing;
    if (item) {
      if (auto it = agent_items_.find(*item); it != agent_items_.end()) {
        existing = it->second;
      }
    } else {
      existing = loose_assistant_;
    }
    if (existing && *existing < messages_.size()) {
      return messages_[*existing];
    }
    messages_.push_back({Role::kAssistant, std::string()});
    const size_t index = messages_.size() - 1;
    if (item) {
      agent_items_[*item] = index;
    } else {
      loose_assistant_ = index;
    }
    return messages_[index];
  }

  void NoteDeclined(const std::string& method) {
    const bool files = method.find("ile") != std::string::npos ||
                       method.find("atch") != std::string::npos;
    messages_.push_back({Role::kTool, std::string("Declined Codex's request to ") +
                                          (files ? "change files"
                                                 : "run a command")});
    loose_assistant_.reset();
    Changed();
  }

  // ---- Tool calls ----

  void StartToolCall(base::Value id, base::DictValue params) {
    const std::optional<std::string> theirs = ThreadIdOf(params);
    const bool same_thread = !theirs || !thread_id_ || *theirs == *thread_id_;
    if (!busy_ || stop_requested_ || !same_thread || !browser_) {
      server_->Respond(id, ToolOutcome::Stopped().RpcResult());
      return;
    }
    const std::string* tool_name = params.FindString("tool");
    const std::string tool = tool_name ? *tool_name : std::string();
    base::DictValue* args_dict = params.FindDict("arguments");
    base::DictValue args = args_dict ? std::move(*args_dict) : base::DictValue();
    messages_.push_back({Role::kTool, RunningLabel(tool, args)});
    const size_t index = messages_.size() - 1;
    // Text after the call goes into a new bubble.
    loose_assistant_.reset();
    status_ = "Using the browser…";
    Changed();
    const uint64_t generation = generation_;
    std::optional<Approval> approval = ApprovalFor(browser_.get(), tool, args);
    if (!approval) {
      RunTool(generation, index, std::move(id), tool, std::move(args));
      return;
    }
    DenyApproval();
    approval_ = approval->text;
    status_ = "Waiting for your approval…";
    approval_reply_ = base::BindOnce(
        [](base::WeakPtr<AssistantSession> session, uint64_t generation,
           size_t index, base::Value id, std::string tool,
           base::DictValue args, bool allow) {
          if (!session) {
            return;
          }
          if (!allow) {
            session->FinishToolCall(generation, index, id,
                                    ToolOutcome::Denied(tool, args));
            return;
          }
          session->RunTool(generation, index, std::move(id), tool,
                           std::move(args));
        },
        weak_factory_.GetWeakPtr(), generation, index, std::move(id), tool,
        std::move(args));
    Changed();
  }

  void RunTool(uint64_t generation,
               size_t index,
               base::Value id,
               const std::string& tool,
               base::DictValue args) {
    if (generation != generation_ || !busy_ || stop_requested_ || !browser_) {
      FinishToolCall(generation, index, id, ToolOutcome::Stopped());
      return;
    }
    ExecuteTool(browser_.get(), tool, std::move(args),
                base::BindOnce(
                    [](base::WeakPtr<AssistantSession> session,
                       uint64_t generation, size_t index, base::Value id,
                       ToolOutcome outcome) {
                      if (session) {
                        session->FinishToolCall(generation, index, id,
                                                std::move(outcome));
                      }
                    },
                    weak_factory_.GetWeakPtr(), generation, index,
                    std::move(id)),
                base::BindRepeating(
                    [](base::WeakPtr<AssistantSession> session,
                       uint64_t generation) {
                      return session && session->generation_ == generation &&
                             session->busy_ && !session->stop_requested_;
                    },
                    weak_factory_.GetWeakPtr(), generation));
  }

  void FinishToolCall(uint64_t generation,
                      size_t index,
                      const base::Value& id,
                      ToolOutcome outcome) {
    const bool current = generation == generation_ && busy_ && !stop_requested_;
    if (!current) {
      outcome = ToolOutcome::Stopped();
    } else if (index < messages_.size()) {
      messages_[index].text = outcome.summary;
    }
    if (server_) {
      server_->Respond(id, outcome.RpcResult());
    }
    if (current) {
      Bump();
      status_ = "Thinking…";
      Changed();
    }
  }

  void DenyApproval() {
    approval_.reset();
    if (approval_reply_) {
      std::move(approval_reply_).Run(false);
    }
  }

  base::WeakPtr<BrowserWindowInterface> browser_;
  base::RepeatingClosure changed_;
  std::vector<Message> messages_;
  bool busy_ = false;
  std::string status_;
  std::optional<std::string> approval_;
  base::OnceCallback<void(bool)> approval_reply_;
  std::string pending_text_;

  std::unique_ptr<CodexServer> server_;
  bool initialized_ = false;
  std::optional<std::string> thread_id_;
  bool uses_tools_ = false;
  bool tools_unsupported_ = false;

  uint64_t generation_ = 0;
  bool stop_requested_ = false;
  std::optional<std::string> active_turn_;
  std::set<std::string> ignored_turns_;
  std::map<std::string, size_t> agent_items_;
  std::optional<size_t> loose_assistant_;
  bool turn_has_text_ = false;
  base::TimeTicks last_activity_;
  base::RepeatingTimer watchdog_;
  base::WeakPtrFactory<AssistantSession> weak_factory_{this};
};

// ---- Panel ------------------------------------------------------------------

bool IsDark(const views::View* view) {
  return view->GetColorProvider() &&
         color_utils::IsDark(
             view->GetColorProvider()->GetColor(ui::kColorFrameActive));
}

gfx::FontList Font(int size, gfx::Font::Weight weight) {
  const gfx::FontList base;
  return base.DeriveWithSizeDelta(size - base.GetFontSize())
      .DeriveWithWeight(weight);
}

SkColor Foreground(const views::View* v) {
  return IsDark(v) ? SkColorSetRGB(0xEE, 0xEE, 0xEF)
                   : SkColorSetRGB(0x1D, 0x1F, 0x22);
}
SkColor Muted(const views::View* v) {
  return IsDark(v) ? SkColorSetRGB(0xA0, 0xA1, 0xA2)
                   : SkColorSetRGB(0x5A, 0x5C, 0x5E);
}
// Mori 3's primary (oklch 0.645 0.11 241 / 0.62 0.13 241.5).
SkColor Primary(const views::View* v) {
  return IsDark(v) ? SkColorSetRGB(0x48, 0x95, 0xCA)
                   : SkColorSetRGB(0x29, 0x8E, 0xCD);
}

// Mori 3's text button: 30px high, 6px corners, 13px medium text; the
// primary one in Mori's blue, the others a faint wash of the text colour.
class TextButton : public views::Button {
  METADATA_HEADER(TextButton, views::Button)

 public:
  TextButton(PressedCallback callback, std::u16string label, bool primary)
      : views::Button(std::move(callback)),
        label_(std::move(label)),
        primary_(primary) {
    SetAccessibleName(label_);
    SetInstallFocusRingOnFocus(true);
  }

  gfx::Size CalculatePreferredSize(
      const views::SizeBounds& available_size) const override {
    return gfx::Size(
        gfx::GetStringWidth(label_, Font(13, gfx::Font::Weight::MEDIUM)) + 24,
        30);
  }

  void PaintButtonContents(gfx::Canvas* canvas) override {
    const bool hovered = GetState() == STATE_HOVERED ||
                         GetState() == STATE_PRESSED;
    const SkColor foreground = Foreground(this);
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(primary_ ? SkColorSetA(Primary(this), hovered ? 0xE6 : 0xFF)
                            : SkColorSetA(foreground, hovered ? 0x1F : 0x12));
    canvas->DrawRoundRect(gfx::RectF(GetLocalBounds()), 6, flags);
    canvas->DrawStringRectWithFlags(
        label_, Font(13, gfx::Font::Weight::MEDIUM),
        primary_ ? SK_ColorWHITE : foreground, GetLocalBounds(),
        gfx::Canvas::TEXT_ALIGN_CENTER);
  }

 private:
  std::u16string label_;
  bool primary_;
};

BEGIN_METADATA(TextButton)
END_METADATA

class InputField : public views::Textfield {
  METADATA_HEADER(InputField, views::Textfield)

 public:
  InputField() {
    SetBorder(views::CreateEmptyBorder(gfx::Insets::VH(7, 10)));
    SetFontList(Font(13, gfx::Font::Weight::NORMAL));
    SetPlaceholderText(u"Ask anything…");
    GetViewAccessibility().SetName(u"Ask the assistant");
  }
  void OnThemeChanged() override {
    views::Textfield::OnThemeChanged();
    SetBackground(views::CreateRoundedRectBackground(
        SkColorSetA(IsDark(this) ? SK_ColorWHITE : SK_ColorBLACK, 0x12), 8));
    SetTextColorId(kColorOmniboxText);
  }
};

BEGIN_METADATA(InputField)
END_METADATA

// The conversation, in the panel's scroll view.
class MessagesView : public views::View {
  METADATA_HEADER(MessagesView, views::View)

 public:
  // The scroll view (bounded) lays it out at its own width, so a message of
  // the user's sits at the right edge and the empty state is centred.
  void ContentsChanged() { PreferredSizeChanged(); }
};

BEGIN_METADATA(MessagesView)
END_METADATA

// StyledLabel owns line fragments. Keep prose selectable like the original
// plain-text messages, while links retain their normal click/focus behavior.
class AssistantMarkdownLabel : public views::StyledLabel {
  METADATA_HEADER(AssistantMarkdownLabel, views::StyledLabel)

 public:
  void Layout(PassKey) override {
    LayoutSuperclass<views::StyledLabel>(this);
    for (views::View* child : children()) {
      if (auto* label = views::AsViewClass<views::Label>(child);
          label && !views::IsViewClass<views::Link>(child)) {
        label->SetSelectable(true);
      }
    }
  }
};

BEGIN_METADATA(AssistantMarkdownLabel)
END_METADATA

class AssistantPanel : public views::View, public views::TextfieldController {
  METADATA_HEADER(AssistantPanel, views::View)

 public:
  explicit AssistantPanel(BrowserView* browser_view)
      : browser_view_(browser_view) {
    session_ = std::make_unique<AssistantSession>(
        browser_view->browser(),
        base::BindRepeating(&AssistantPanel::ScheduleRebuild,
                            base::Unretained(this)));
    auto* layout = SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical));
    // Header: sparkles, "Assistant", new conversation, close.
    auto* header = AddChildView(std::make_unique<views::BoxLayoutView>());
    header->SetInsideBorderInsets(gfx::Insets::VH(0, 12));
    header->SetBetweenChildSpacing(8);
    header->SetCrossAxisAlignment(views::BoxLayout::CrossAxisAlignment::kCenter);
    header->SetPreferredSize(gfx::Size(0, 48));
    sparkles_ = header->AddChildView(std::make_unique<views::ImageView>());
    title_ = header->AddChildView(std::make_unique<views::Label>(u"Assistant"));
    title_->SetFontList(Font(14, gfx::Font::Weight::SEMIBOLD));
    title_->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    header->SetFlexForView(title_, 1);
    new_chat_ = header->AddChildView(views::CreateVectorImageButton(
        base::BindRepeating(&AssistantPanel::NewChat, base::Unretained(this))));
    new_chat_->GetViewAccessibility().SetName(u"New conversation");
    close_ = header->AddChildView(views::CreateVectorImageButton(
        base::BindRepeating(
            [](BrowserView* view) { ToggleAssistant(view->browser()); },
            browser_view)));
    close_->GetViewAccessibility().SetName(u"Close assistant");
    separator_ = AddChildView(std::make_unique<views::Separator>());

    scroll_ = AddChildView(std::make_unique<views::ScrollView>());
    scroll_->SetBackgroundColor(std::nullopt);
    scroll_->SetHorizontalScrollBarMode(
        views::ScrollView::ScrollBarMode::kDisabled);
    // Bounded, the scroll view gives its contents its own width.
    scroll_->ClipHeightTo(0, std::numeric_limits<int>::max());
    content_ = scroll_->SetContents(std::make_unique<MessagesView>());
    content_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(12), 12));
    layout->SetFlexForView(scroll_, 1);

    approval_ = AddChildView(std::make_unique<views::View>());
    approval_->SetLayoutManager(std::make_unique<views::BoxLayout>(
        views::BoxLayout::Orientation::kVertical, gfx::Insets(12), 8));
    approval_->SetProperty(views::kMarginsKey, gfx::Insets::VH(0, 12));

    auto* input_row = AddChildView(std::make_unique<views::BoxLayoutView>());
    input_row->SetInsideBorderInsets(gfx::Insets(12));
    input_row->SetBetweenChildSpacing(8);
    input_row->SetCrossAxisAlignment(
        views::BoxLayout::CrossAxisAlignment::kCenter);
    input_ = input_row->AddChildView(std::make_unique<InputField>());
    input_->set_controller(this);
    input_row->SetFlexForView(input_, 1);
    stop_ = input_row->AddChildView(views::CreateVectorImageButton(
        base::BindRepeating(&AssistantPanel::Stop, base::Unretained(this))));
    stop_->GetViewAccessibility().SetName(u"Stop");
    Rebuild();
  }

  void FocusInput() { input_->RequestFocus(); }

  void Ask(const std::string& prompt) { session_->Send(prompt); }

  // views::TextfieldController:
  bool HandleKeyEvent(views::Textfield* sender,
                      const ui::KeyEvent& event) override {
    if (event.type() == ui::EventType::kKeyPressed &&
        event.key_code() == ui::VKEY_RETURN && !event.IsShiftDown()) {
      const std::string text = base::UTF16ToUTF8(input_->GetText());
      input_->SetText(u"");
      session_->Send(text);
      return true;
    }
    return false;
  }

  void OnThemeChanged() override {
    views::View::OnThemeChanged();
    sparkles_->SetImage(ui::ImageModel::FromVectorIcon(LucideIcon("sparkles"),
                                                       Primary(this), 16));
    auto set_icon = [](views::ImageButton* button, std::string_view icon,
                       int size, SkColor color) {
      button->SetImageModel(views::Button::STATE_NORMAL,
                            ui::ImageModel::FromVectorIcon(LucideIcon(icon),
                                                           color, size));
    };
    set_icon(new_chat_, "square-pen", 15, Muted(this));
    set_icon(close_, "x", 15, Muted(this));
    set_icon(stop_, "square", 14, Foreground(this));
    title_->SetEnabledColor(Foreground(this));
    Rebuild();
  }

  // Mori 3's card: the card colour with a soft edge and shadow.
  void OnPaintBackground(gfx::Canvas* canvas) override {
    const bool dark = IsDark(this);
    const gfx::RectF bounds(GetLocalBounds());
    cc::PaintFlags flags;
    flags.setAntiAlias(true);
    flags.setColor(dark ? SkColorSetRGB(0x2B, 0x2B, 0x2B)
                        : SkColorSetRGB(0xFB, 0xFB, 0xFB));
    canvas->DrawRoundRect(bounds, 6, flags);
    cc::PaintFlags edge;
    edge.setAntiAlias(true);
    edge.setStyle(cc::PaintFlags::kStroke_Style);
    edge.setStrokeWidth(1);
    edge.setColor(SkColorSetA(dark ? SK_ColorWHITE : SK_ColorBLACK, 0x14));
    gfx::RectF inner = bounds;
    inner.Inset(0.5f);
    canvas->DrawRoundRect(inner, 5.5f, edge);
  }

 private:
  void NewChat() { session_->Reset(); }
  void Stop() { session_->Interrupt(); }

  // Coalesces bursts of streamed text into one rebuild.
  void ScheduleRebuild() {
    if (rebuild_pending_) {
      return;
    }
    rebuild_pending_ = true;
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(&AssistantPanel::Rebuild,
                                  weak_factory_.GetWeakPtr()));
  }

  views::Label* AddText(views::View* parent,
                        const std::string& text,
                        int size,
                        SkColor color) {
    auto* label = parent->AddChildView(
        std::make_unique<views::Label>(base::UTF8ToUTF16(text)));
    label->SetMultiLine(true);
    label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
    label->SetFontList(Font(size, gfx::Font::Weight::NORMAL));
    // Mori 3's line height (the font list's own is far looser).
    label->SetLineHeight(size * 3 / 2);
    label->SetEnabledColor(color);
    label->SetSelectable(true);
    label->SetMaximumWidth(kAssistantPanelWidth - 24 - 24);
    return label;
  }

  void AddMarkdown(const std::string& markdown_text) {
    auto* message = content_->AddChildView(
        std::make_unique<views::BoxLayoutView>());
    message->SetOrientation(views::BoxLayout::Orientation::kVertical);
    message->SetCrossAxisAlignment(
        views::BoxLayout::CrossAxisAlignment::kStretch);
    message->SetBetweenChildSpacing(6);
    for (const markdown::Block& block : markdown::Parse(markdown_text)) {
      if (block.kind == markdown::Block::kRule) {
        message->AddChildView(std::make_unique<views::Separator>());
        continue;
      }
      if (block.kind == markdown::Block::kCode) {
        const std::string code = block.spans.empty() ? "" : block.spans[0].text;
        auto* label = AddText(message, code, 12, Foreground(this));
        label->SetFontList(gfx::FontList(
            {"Menlo", "Consolas", "monospace"}, gfx::Font::NORMAL, 12,
            gfx::Font::Weight::NORMAL));
        const SkColor background = IsDark(this)
                                       ? SkColorSetRGB(0x25, 0x25, 0x27)
                                       : SkColorSetRGB(0xEF, 0xEF, 0xF1);
        label->SetBackgroundColor(background);
        label->SetBackground(views::CreateRoundedRectBackground(background, 6));
        label->SetBorder(views::CreateEmptyBorder(gfx::Insets(8)));
        continue;
      }
      auto spans = block.spans;
      if (!block.prefix.empty()) {
        spans.insert(spans.begin(), markdown::Span{block.prefix, false, false, false, {}});
      }
      std::u16string text;
      for (const auto& span : spans) {
        text += base::UTF8ToUTF16(span.text);
      }
      auto* label = message->AddChildView(std::make_unique<AssistantMarkdownLabel>());
      label->SetText(std::move(text));
      const int size = block.kind == markdown::Block::kHeading
                           ? (block.level == 1 ? 18 : block.level == 2 ? 16 : 14)
                           : 13;
      label->SetLineHeight(size * 3 / 2);
      label->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      label->SetDisplayedOnBackgroundColor(
          IsDark(this) ? SkColorSetRGB(0x16, 0x16, 0x17)
                       : SkColorSetRGB(0xFA, 0xFA, 0xFA));
      if (block.kind == markdown::Block::kQuote) {
        label->SetBorder(views::CreateSolidSidedBorder(
            gfx::Insets::TLBR(0, 2, 0, 0), Muted(this)));
      }
      size_t offset = 0;
      for (const auto& span : spans) {
        const size_t end = std::min(
            offset + base::UTF8ToUTF16(span.text).size(), label->GetText().size());
        if (offset >= end) {
          break;
        }
        views::StyledLabel::RangeStyleInfo style;
        const GURL url(span.link);
        if (url.is_valid() && url.SchemeIsHTTPOrHTTPS()) {
          style = views::StyledLabel::RangeStyleInfo::CreateForLink(
              base::BindRepeating(
                  [](base::WeakPtr<BrowserWindowInterface> browser, GURL url) {
                    if (browser) {
                      chrome::AddTabAt(browser.get(), url, -1, true);
                    }
                  },
                  browser_view_->browser()->GetWeakPtr(), url));
          style.tooltip = base::UTF8ToUTF16(url.spec());
          style.override_color = IsDark(this)
                                     ? SkColorSetRGB(0x8A, 0xB4, 0xF8)
                                     : SkColorSetRGB(0x19, 0x67, 0xD2);
        } else {
          style.override_color = block.kind == markdown::Block::kQuote
                                     ? Muted(this) : Foreground(this);
        }
        gfx::FontList font = span.code
            ? gfx::FontList({"Menlo", "Consolas", "monospace"},
                            gfx::Font::NORMAL, size, gfx::Font::Weight::NORMAL)
            : Font(size, gfx::Font::Weight::NORMAL);
        if (span.bold || block.kind == markdown::Block::kHeading) {
          font = font.DeriveWithWeight(gfx::Font::Weight::SEMIBOLD);
        }
        if (span.italic) {
          font = font.DeriveWithStyle(gfx::Font::ITALIC);
        }
        style.custom_font = font;
        label->AddStyleRange(gfx::Range(offset, end), style);
        offset = end;
      }
      label->SizeToFit(kAssistantPanelWidth - 48);
    }
  }

  void Rebuild() {
    rebuild_pending_ = false;
    if (!GetColorProvider()) {
      return;
    }
    const bool dark = IsDark(this);
    content_->RemoveAllChildViews();
    const std::vector<Message>& messages = session_->messages();
    if (messages.empty()) {
      // Mori 3's empty state: a title and three starters.
      auto* empty = content_->AddChildView(std::make_unique<views::BoxLayoutView>());
      empty->SetOrientation(views::BoxLayout::Orientation::kVertical);
      empty->SetCrossAxisAlignment(
          views::BoxLayout::CrossAxisAlignment::kCenter);
      empty->SetBetweenChildSpacing(10);
      empty->SetInsideBorderInsets(gfx::Insets::TLBR(32, 0, 0, 0));
      auto* title = empty->AddChildView(
          std::make_unique<views::Label>(u"Ask about this page"));
      title->SetFontList(Font(15, gfx::Font::Weight::NORMAL));
      title->SetEnabledColor(Muted(this));
      auto* chips = empty->AddChildView(std::make_unique<views::BoxLayoutView>());
      chips->SetBetweenChildSpacing(6);
      for (auto [label, prompt] :
           {std::pair{u"Summarize", "Summarize this page."},
            std::pair{u"Key points", "List the key points of this page."},
            std::pair{u"Tidy tabs",
                      "Tidy up my tabs: suggest folders and close "
                      "duplicates."}}) {
        chips->AddChildView(std::make_unique<TextButton>(
            base::BindRepeating(
                [](AssistantSession* session, std::string prompt) {
                  session->Send(prompt);
                },
                base::Unretained(session_.get()), std::string(prompt)),
            label, /*primary=*/false));
      }
    }
    for (const Message& message : messages) {
      switch (message.role) {
        case Role::kUser: {
          // A bubble at the end of the row.
          auto* row = content_->AddChildView(std::make_unique<views::BoxLayoutView>());
          row->SetMainAxisAlignment(views::BoxLayout::MainAxisAlignment::kEnd);
          auto* bubble = row->AddChildView(std::make_unique<views::View>());
          bubble->SetLayoutManager(std::make_unique<views::BoxLayout>(
              views::BoxLayout::Orientation::kVertical,
              gfx::Insets::VH(7, 10)));
          bubble->SetBackground(views::CreateRoundedRectBackground(
              SkColorSetA(Primary(this), 0x29), 10));
          AddText(bubble, message.text, 13, Foreground(this))
              ->SetMaximumWidth(kAssistantPanelWidth - 24 - 60);
          break;
        }
        case Role::kAssistant:
          AddMarkdown(message.text);
          break;
        case Role::kTool:
          AddText(content_, message.text, 12, Muted(this));
          break;
        case Role::kError: {
          auto* box = content_->AddChildView(std::make_unique<views::View>());
          box->SetLayoutManager(std::make_unique<views::BoxLayout>(
              views::BoxLayout::Orientation::kVertical,
              gfx::Insets::VH(7, 10)));
          box->SetBackground(views::CreateRoundedRectBackground(
              SkColorSetA(SkColorSetRGB(0xE5, 0x48, 0x4D), 0x1F), 8));
          AddText(box, message.text, 12,
                  dark ? SkColorSetRGB(0xFF, 0x9E, 0x9E)
                       : SkColorSetRGB(0xB4, 0x23, 0x18));
          break;
        }
      }
    }
    if (!session_->status().empty()) {
      AddText(content_, session_->status(), 12, Muted(this));
    }
    // The approval card.
    approval_->RemoveAllChildViews();
    approval_->SetVisible(session_->approval().has_value());
    if (session_->approval()) {
      const SkColor warning = SkColorSetRGB(0xE6, 0xA2, 0x3C);
      approval_->SetBackground(views::CreateRoundedRectBackground(
          SkColorSetA(warning, 0x14), 10));
      approval_->SetBorder(views::CreateRoundedRectBorder(
          1, 10, SkColorSetA(warning, 0x99)));
      auto* heading = approval_->AddChildView(
          std::make_unique<views::Label>(u"Allow this action?"));
      heading->SetFontList(Font(13, gfx::Font::Weight::MEDIUM));
      heading->SetHorizontalAlignment(gfx::ALIGN_LEFT);
      heading->SetEnabledColor(Foreground(this));
      AddText(approval_, *session_->approval(), 12, Foreground(this))
          ->SetMaximumWidth(kAssistantPanelWidth - 24 - 24 - 2);
      auto* buttons = approval_->AddChildView(std::make_unique<views::BoxLayoutView>());
      buttons->SetMainAxisAlignment(views::BoxLayout::MainAxisAlignment::kEnd);
      buttons->SetBetweenChildSpacing(8);
      buttons->AddChildView(std::make_unique<TextButton>(
          base::BindRepeating(&AssistantSession::ResolveApproval,
                              base::Unretained(session_.get()), false),
          u"Deny", /*primary=*/false));
      buttons->AddChildView(std::make_unique<TextButton>(
          base::BindRepeating(&AssistantSession::ResolveApproval,
                              base::Unretained(session_.get()), true),
          u"Allow", /*primary=*/true));
    }
    stop_->SetVisible(session_->busy());
    content_->ContentsChanged();
    InvalidateLayout();
    // Newest at the bottom, in view.
    base::SingleThreadTaskRunner::GetCurrentDefault()->PostTask(
        FROM_HERE, base::BindOnce(
                       [](base::WeakPtr<AssistantPanel> panel) {
                         if (panel) {
                           panel->content_->ScrollRectToVisible(gfx::Rect(
                               0, panel->content_->height() - 1, 1, 1));
                         }
                       },
                       weak_factory_.GetWeakPtr()));
  }

  raw_ptr<BrowserView> browser_view_;
  std::unique_ptr<AssistantSession> session_;
  raw_ptr<views::ImageView> sparkles_ = nullptr;
  raw_ptr<views::Label> title_ = nullptr;
  raw_ptr<views::ImageButton> new_chat_ = nullptr;
  raw_ptr<views::ImageButton> close_ = nullptr;
  raw_ptr<views::Separator> separator_ = nullptr;
  raw_ptr<views::ScrollView> scroll_ = nullptr;
  raw_ptr<MessagesView> content_ = nullptr;
  raw_ptr<views::View> approval_ = nullptr;
  raw_ptr<InputField> input_ = nullptr;
  raw_ptr<views::ImageButton> stop_ = nullptr;
  bool rebuild_pending_ = false;
  base::WeakPtrFactory<AssistantPanel> weak_factory_{this};
};

BEGIN_METADATA(AssistantPanel)
END_METADATA

AssistantPanel* FindPanel(views::View* browser_view) {
  for (views::View* child : browser_view->children()) {
    if (auto* panel = views::AsViewClass<AssistantPanel>(child)) {
      return panel;
    }
  }
  return nullptr;
}

}  // namespace

void ToggleAssistant(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (!browser_view) {
    return;
  }
  AssistantPanel* panel = FindPanel(browser_view);
  if (!panel) {
    panel = browser_view->AddChildView(
        std::make_unique<AssistantPanel>(browser_view));
    panel->SetVisible(false);
  }
  const bool open = !panel->GetVisible();
  panel->SetVisible(open);
  browser_view->InvalidateLayout();
  if (open) {
    panel->FocusInput();
  } else if (content::WebContents* contents =
                 browser->GetTabStripModel()->GetActiveWebContents()) {
    contents->Focus();
  }
}

void AskAssistant(BrowserWindowInterface* browser, const std::string& prompt) {
  if (!IsAssistantOpen(browser)) {
    ToggleAssistant(browser);
  }
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  if (AssistantPanel* panel = browser_view ? FindPanel(browser_view) : nullptr) {
    panel->Ask(prompt);
  }
}

bool IsAssistantOpen(BrowserWindowInterface* browser) {
  BrowserView* browser_view = BrowserView::GetBrowserViewForBrowser(browser);
  AssistantPanel* panel = browser_view ? FindPanel(browser_view) : nullptr;
  return panel && panel->GetVisible();
}

views::View* GetVisibleAssistantPanel(views::View* browser_view) {
  AssistantPanel* panel = FindPanel(browser_view);
  return panel && panel->GetVisible() ? panel : nullptr;
}

}  // namespace mori

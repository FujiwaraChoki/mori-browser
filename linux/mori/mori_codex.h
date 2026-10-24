// The local Codex app server (`codex app-server`) that Mori's assistant
// talks to, as Mori does: JSON-RPC lines over the server's stdin and
// stdout. Requests (`{id, method, params}`), responses, notifications and
// the server's own requests (tool calls, approvals).
//
// All process and pipe IO happens off the UI thread; replies and events
// come back on it. Destroying the server stops the process (its process
// group: SIGTERM, then SIGKILL).

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_CODEX_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_CODEX_H_

#include <map>
#include <memory>
#include <optional>
#include <string>

#include "base/files/file_path.h"
#include "base/functional/callback.h"
#include "base/memory/raw_ptr.h"
#include "base/memory/scoped_refptr.h"
#include "base/memory/weak_ptr.h"
#include "base/time/time.h"
#include "base/values.h"

namespace mori {

// Where `codex` is installed, if it is (blocking: checks files).
std::optional<base::FilePath> FindCodexBinary();

class CodexServer {
 public:
  class Delegate {
   public:
    virtual void OnCodexNotification(const std::string& method,
                                     const base::DictValue& params) = 0;
    virtual void OnCodexRequest(base::Value id,
                                const std::string& method,
                                base::DictValue params) = 0;
    virtual void OnCodexClosed(const std::string& reason) = 0;

   protected:
    virtual ~Delegate() = default;
  };

  // A request's result, or its error message.
  using Reply = base::OnceCallback<void(std::optional<base::Value> result,
                                        std::string error)>;

  // Starts `codex app-server` in `working_dir`. Failing to start it shows
  // up as OnCodexClosed.
  CodexServer(base::FilePath working_dir, Delegate* delegate);
  ~CodexServer();

  void Request(const std::string& method,
               base::DictValue params,
               base::TimeDelta timeout,
               Reply reply);
  void Notify(const std::string& method, base::DictValue params);
  void Respond(const base::Value& id, base::Value result);
  void RespondError(const base::Value& id, int code, const std::string& message);

 private:
  class Io;

  void OnLine(std::string line);
  void OnClosed(std::string reason);
  void OnTimeout(int id, std::string method);
  void Send(base::DictValue frame);

  raw_ptr<Delegate> delegate_;
  scoped_refptr<Io> io_;
  int next_id_ = 1;
  std::map<int, Reply> pending_;
  bool closed_ = false;
  base::WeakPtrFactory<CodexServer> weak_factory_{this};
};

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_CODEX_H_

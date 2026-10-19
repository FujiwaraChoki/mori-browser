#include "chrome/browser/ui/views/mori/mori_codex.h"

#include <fcntl.h>
#include <pwd.h>
#include <signal.h>
#include <unistd.h>

#include <utility>
#include <vector>

#include "base/command_line.h"
#include "base/environment.h"
#include "base/files/file_util.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/memory/ref_counted.h"
#include "base/path_service.h"
#include "base/posix/eintr_wrapper.h"
#include "base/process/launch.h"
#include "base/process/process.h"
#include "base/strings/string_split.h"
#include "base/task/sequenced_task_runner.h"
#include "base/task/single_thread_task_runner.h"
#include "base/task/thread_pool.h"
#include "base/threading/platform_thread.h"

namespace mori {
namespace {

// $HOME, or the account's home when the browser was started without one.
base::FilePath HomeDir() {
  auto env = base::Environment::Create();
  if (std::optional<std::string> home = env->GetVar("HOME");
      home && !home->empty()) {
    return base::FilePath(*home);
  }
  if (const passwd* entry = getpwuid(getuid()); entry && entry->pw_dir) {
    return base::FilePath(entry->pw_dir);
  }
  return base::FilePath();
}

// Where to look for `codex`, as Mori 3 does: $CODEX_BIN, $PATH, then the
// per-user install places a desktop-launched browser may not have on its
// PATH (npm, bun, mise).
std::optional<base::FilePath> FindCodex() {
  auto env = base::Environment::Create();
  std::vector<base::FilePath> candidates;
  if (std::optional<std::string> bin = env->GetVar("CODEX_BIN");
      bin && !bin->empty()) {
    candidates.emplace_back(*bin);
  }
  if (std::optional<std::string> path = env->GetVar("PATH")) {
    for (const std::string& dir : base::SplitString(
             *path, ":", base::TRIM_WHITESPACE, base::SPLIT_WANT_NONEMPTY)) {
      candidates.push_back(base::FilePath(dir).Append("codex"));
    }
  }
  if (const base::FilePath home = HomeDir(); !home.empty()) {
    for (const char* dir :
         {".local/bin", ".bun/bin", ".npm-global/bin",
          ".local/share/mise/installs/codex/latest/bin"}) {
      candidates.push_back(home.AppendASCII(dir).Append("codex"));
    }
  }
  // Homebrew on Apple silicon (apps opened from the Dock don't get the
  // shell's PATH), then the system places.
  candidates.emplace_back("/opt/homebrew/bin/codex");
  candidates.emplace_back("/usr/local/bin/codex");
  candidates.emplace_back("/usr/bin/codex");
  for (const base::FilePath& candidate : candidates) {
    if (access(candidate.value().c_str(), X_OK) == 0 &&
        !base::DirectoryExists(candidate)) {
      return candidate;
    }
  }
  return std::nullopt;
}

// pipe2(O_CLOEXEC) where there is one (not macOS).
bool CloseOnExecPipe(int fds[2]) {
  if (pipe(fds) != 0) {
    return false;
  }
  for (int i = 0; i < 2; ++i) {
    if (fcntl(fds[i], F_SETFD, FD_CLOEXEC) != 0) {
      close(fds[0]);
      close(fds[1]);
      return false;
    }
  }
  return true;
}

}  // namespace

std::optional<base::FilePath> FindCodexBinary() {
  return FindCodex();
}

// The process and its pipes. Writes and shutdown run on `runner_`; a
// dedicated thread reads the server's stdout and posts each line back.
class CodexServer::Io : public base::RefCountedThreadSafe<CodexServer::Io> {
 public:
  Io(scoped_refptr<base::SequencedTaskRunner> ui,
     base::WeakPtr<CodexServer> server)
      : ui_(std::move(ui)),
        server_(std::move(server)),
        runner_(base::ThreadPool::CreateSequencedTaskRunner(
            {base::MayBlock(), base::TaskPriority::USER_BLOCKING,
             base::TaskShutdownBehavior::CONTINUE_ON_SHUTDOWN})),
        reader_(base::ThreadPool::CreateSingleThreadTaskRunner(
            {base::MayBlock(), base::TaskShutdownBehavior::CONTINUE_ON_SHUTDOWN},
            base::SingleThreadTaskRunnerThreadMode::DEDICATED)) {}

  void Start(base::FilePath working_dir) {
    runner_->PostTask(FROM_HERE,
                      base::BindOnce(&Io::Launch, this, std::move(working_dir)));
  }

  void Write(std::string line) {
    runner_->PostTask(FROM_HERE,
                      base::BindOnce(&Io::WriteOnRunner, this, std::move(line)));
  }

  void Stop() { runner_->PostTask(FROM_HERE, base::BindOnce(&Io::Kill, this)); }

 private:
  friend class base::RefCountedThreadSafe<Io>;
  ~Io() = default;

  void Closed(std::string reason) {
    ui_->PostTask(FROM_HERE, base::BindOnce(&CodexServer::OnClosed, server_,
                                            std::move(reason)));
  }

  void Launch(base::FilePath working_dir) {
    std::optional<base::FilePath> codex = FindCodex();
    if (!codex) {
      Closed(
          "Mori couldn't find Codex. Install the Codex CLI (npm install -g "
          "@openai/codex) and sign in with `codex login`.");
      return;
    }
    base::CreateDirectory(working_dir);
    int in[2];
    int out[2];
    if (!CloseOnExecPipe(in)) {
      Closed("Couldn't start Codex (no pipes).");
      return;
    }
    if (!CloseOnExecPipe(out)) {
      close(in[0]);
      close(in[1]);
      Closed("Couldn't start Codex (no pipes).");
      return;
    }
    // Its stderr goes to a log next to its working directory.
    base::File log(working_dir.AppendASCII("codex-app-server.log"),
                   base::File::FLAG_CREATE_ALWAYS | base::File::FLAG_WRITE);
    base::LaunchOptions options;
    options.current_directory = working_dir;
    options.new_process_group = true;
    options.fds_to_remap.emplace_back(in[0], STDIN_FILENO);
    options.fds_to_remap.emplace_back(out[1], STDOUT_FILENO);
    if (log.IsValid()) {
      options.fds_to_remap.emplace_back(log.GetPlatformFile(), STDERR_FILENO);
    }
    // Codex finds its sign-in under $HOME.
    if (const base::FilePath home = HomeDir(); !home.empty()) {
      options.environment["HOME"] = home.value();
    }
    // Codex's own directory first: an npm install is a script that needs
    // the `node` next to it, which a desktop-launched browser's PATH (the
    // Dock's, on macOS) may not have.
    {
      std::string path = codex->DirName().value();
      if (std::optional<std::string> inherited =
              base::Environment::Create()->GetVar("PATH");
          inherited && !inherited->empty()) {
        path += ":" + *inherited;
      } else {
        path += ":/usr/local/bin:/usr/bin:/bin";
      }
      options.environment["PATH"] = path;
    }
    base::CommandLine command(*codex);
    command.AppendArg("app-server");
    process_ = base::LaunchProcess(command, options);
    close(in[0]);
    close(out[1]);
    if (!process_.IsValid()) {
      close(in[1]);
      close(out[0]);
      Closed("Couldn't start Codex.");
      return;
    }
    stdin_ = in[1];
    reader_->PostTask(FROM_HERE, base::BindOnce(&Io::ReadLoop, this, out[0]));
    for (std::string& line : queued_) {
      WriteOnRunner(std::move(line));
    }
    queued_.clear();
  }

  // Blocks on the server's stdout until it closes.
  void ReadLoop(int fd) {
    std::string buffer;
    char chunk[16384];
    while (true) {
      const ssize_t n = HANDLE_EINTR(read(fd, chunk, sizeof(chunk)));
      if (n <= 0) {
        break;
      }
      buffer.append(chunk, static_cast<size_t>(n));
      size_t newline;
      while ((newline = buffer.find('\n')) != std::string::npos) {
        std::string line = buffer.substr(0, newline);
        buffer.erase(0, newline + 1);
        if (!line.empty()) {
          ui_->PostTask(FROM_HERE, base::BindOnce(&CodexServer::OnLine, server_,
                                                  std::move(line)));
        }
      }
    }
    close(fd);
    Closed("The local Codex app server disconnected.");
  }

  void WriteOnRunner(std::string line) {
    if (stdin_ < 0) {
      if (!process_.IsValid()) {
        queued_.push_back(std::move(line));
      }
      return;
    }
    line.push_back('\n');
    const char* data = line.data();
    size_t left = line.size();
    while (left > 0) {
      const ssize_t n = HANDLE_EINTR(write(stdin_, data, left));
      if (n <= 0) {
        return;
      }
      data += n;
      left -= static_cast<size_t>(n);
    }
  }

  void Kill() {
    if (stdin_ >= 0) {
      close(stdin_);
      stdin_ = -1;
    }
    if (!process_.IsValid()) {
      return;
    }
    // The whole group: codex may have started helpers.
    kill(-process_.Pid(), SIGTERM);
    for (int i = 0; i < 20; ++i) {
      int exit_code = 0;
      if (process_.WaitForExitWithTimeout(base::Milliseconds(100),
                                          &exit_code)) {
        return;
      }
    }
    kill(-process_.Pid(), SIGKILL);
    process_.WaitForExitWithTimeout(base::Seconds(1), nullptr);
  }

  scoped_refptr<base::SequencedTaskRunner> ui_;
  base::WeakPtr<CodexServer> server_;
  scoped_refptr<base::SequencedTaskRunner> runner_;
  scoped_refptr<base::SingleThreadTaskRunner> reader_;
  base::Process process_;
  int stdin_ = -1;
  std::vector<std::string> queued_;
};

CodexServer::CodexServer(base::FilePath working_dir, Delegate* delegate)
    : delegate_(delegate) {
  io_ = base::MakeRefCounted<Io>(
      base::SequencedTaskRunner::GetCurrentDefault(),
      weak_factory_.GetWeakPtr());
  io_->Start(std::move(working_dir));
}

CodexServer::~CodexServer() {
  io_->Stop();
}

void CodexServer::Request(const std::string& method,
                          base::DictValue params,
                          base::TimeDelta timeout,
                          Reply reply) {
  if (closed_) {
    std::move(reply).Run(std::nullopt,
                         "The local Codex app server disconnected.");
    return;
  }
  const int id = next_id_++;
  pending_[id] = std::move(reply);
  Send(base::DictValue()
           .Set("id", id)
           .Set("method", method)
           .Set("params", std::move(params)));
  base::SequencedTaskRunner::GetCurrentDefault()->PostDelayedTask(
      FROM_HERE,
      base::BindOnce(&CodexServer::OnTimeout, weak_factory_.GetWeakPtr(), id,
                     method),
      timeout);
}

void CodexServer::Notify(const std::string& method, base::DictValue params) {
  Send(base::DictValue().Set("method", method).Set("params", std::move(params)));
}

void CodexServer::Respond(const base::Value& id, base::Value result) {
  Send(base::DictValue().Set("id", id.Clone()).Set("result", std::move(result)));
}

void CodexServer::RespondError(const base::Value& id,
                               int code,
                               const std::string& message) {
  Send(base::DictValue()
           .Set("id", id.Clone())
           .Set("error",
                base::DictValue().Set("code", code).Set("message", message)));
}

void CodexServer::Send(base::DictValue frame) {
  std::string line;
  base::JSONWriter::Write(frame, &line);
  io_->Write(std::move(line));
}

void CodexServer::OnLine(std::string line) {
  std::optional<base::Value> value =
      base::JSONReader::Read(line, base::JSON_PARSE_RFC);
  if (!value || !value->is_dict()) {
    return;
  }
  base::DictValue& frame = value->GetDict();
  const std::string* method = frame.FindString("method");
  base::Value* id = frame.Find("id");
  if (method) {
    base::DictValue* params = frame.FindDict("params");
    base::DictValue owned = params ? std::move(*params) : base::DictValue();
    if (id) {
      delegate_->OnCodexRequest(id->Clone(), *method, std::move(owned));
    } else {
      delegate_->OnCodexNotification(*method, owned);
    }
    return;
  }
  if (!id || !id->is_int()) {
    return;
  }
  auto it = pending_.find(id->GetInt());
  if (it == pending_.end()) {
    return;
  }
  Reply reply = std::move(it->second);
  pending_.erase(it);
  if (const base::DictValue* error = frame.FindDict("error")) {
    const std::string* message = error->FindString("message");
    std::move(reply).Run(std::nullopt,
                         message ? *message : "Codex reported an error.");
    return;
  }
  base::Value* result = frame.Find("result");
  std::move(reply).Run(result ? std::move(*result) : base::Value(),
                       std::string());
}

void CodexServer::OnTimeout(int id, std::string method) {
  auto it = pending_.find(id);
  if (it == pending_.end()) {
    return;
  }
  Reply reply = std::move(it->second);
  pending_.erase(it);
  std::move(reply).Run(
      std::nullopt,
      "Timed out waiting for the Codex app server (" + method + ").");
}

void CodexServer::OnClosed(std::string reason) {
  if (closed_) {
    return;
  }
  closed_ = true;
  std::map<int, Reply> pending;
  pending.swap(pending_);
  for (auto& [id, reply] : pending) {
    std::move(reply).Run(std::nullopt, reason);
  }
  delegate_->OnCodexClosed(reason);
}

}  // namespace mori

#include "chrome/browser/ui/views/mori/mori_codex.h"

#include <utility>
#include <vector>

#include "base/command_line.h"
#include "base/containers/span.h"
#include "base/environment.h"
#include "base/files/file.h"
#include "base/files/file_util.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/memory/ref_counted.h"
#include "base/path_service.h"
#include "base/process/launch.h"
#include "base/process/process.h"
#include "base/strings/string_split.h"
#include "base/strings/string_util.h"
#include "base/task/sequenced_task_runner.h"
#include "base/task/single_thread_task_runner.h"
#include "base/task/thread_pool.h"
#include "base/threading/platform_thread.h"
#include "build/build_config.h"

#if BUILDFLAG(IS_WIN)
#include <windows.h>

#include "base/base_paths_win.h"
#include "base/win/scoped_handle.h"
#else
#include <fcntl.h>
#include <pwd.h>
#include <signal.h>
#include <unistd.h>
#endif

namespace mori {
namespace {

#if BUILDFLAG(IS_WIN)
constexpr char kPathSeparator[] = ";";
#else
constexpr char kPathSeparator[] = ":";
#endif

// The account's home directory ($HOME, or the account's when the browser
// was started without one; %USERPROFILE% on Windows).
base::FilePath HomeDir() {
#if BUILDFLAG(IS_WIN)
  base::FilePath home;
  base::PathService::Get(base::DIR_HOME, &home);
  return home;
#else
  auto env = base::Environment::Create();
  if (std::optional<std::string> home = env->GetVar("HOME");
      home && !home->empty()) {
    return base::FilePath(*home);
  }
  if (const passwd* entry = getpwuid(getuid()); entry && entry->pw_dir) {
    return base::FilePath(entry->pw_dir);
  }
  return base::FilePath();
#endif
}

bool IsRunnable(const base::FilePath& path) {
#if BUILDFLAG(IS_WIN)
  return base::PathExists(path) && !base::DirectoryExists(path);
#else
  return access(path.value().c_str(), X_OK) == 0 &&
         !base::DirectoryExists(path);
#endif
}

// Where to look for `codex`, as Mori 3 does: $CODEX_BIN, $PATH, then the
// per-user install places a desktop-launched browser may not have on its
// PATH (npm, bun, mise; Homebrew; on Windows npm, Scoop and WinGet).
std::optional<base::FilePath> FindCodex() {
  auto env = base::Environment::Create();
  std::vector<base::FilePath> candidates;
  if (std::optional<std::string> bin = env->GetVar("CODEX_BIN");
      bin && !bin->empty()) {
    candidates.push_back(base::FilePath::FromUTF8Unsafe(*bin));
  }
#if BUILDFLAG(IS_WIN)
  const std::vector<const char*> names = {"codex.exe", "codex.cmd"};
#else
  const std::vector<const char*> names = {"codex"};
#endif
  if (std::optional<std::string> path = env->GetVar("PATH")) {
    for (const std::string& dir :
         base::SplitString(*path, kPathSeparator, base::TRIM_WHITESPACE,
                           base::SPLIT_WANT_NONEMPTY)) {
      for (const char* name : names) {
        candidates.push_back(
            base::FilePath::FromUTF8Unsafe(dir).AppendASCII(name));
      }
    }
  }
  const base::FilePath home = HomeDir();
#if BUILDFLAG(IS_WIN)
  base::FilePath roaming;
  if (base::PathService::Get(base::DIR_ROAMING_APP_DATA, &roaming)) {
    candidates.push_back(roaming.AppendASCII("npm").AppendASCII("codex.cmd"));
  }
  base::FilePath local;
  if (base::PathService::Get(base::DIR_LOCAL_APP_DATA, &local)) {
    candidates.push_back(local.AppendASCII("Microsoft")
                             .AppendASCII("WinGet")
                             .AppendASCII("Links")
                             .AppendASCII("codex.exe"));
  }
  if (!home.empty()) {
    candidates.push_back(
        home.AppendASCII("scoop").AppendASCII("shims").AppendASCII(
            "codex.exe"));
    candidates.push_back(
        home.AppendASCII(".local").AppendASCII("bin").AppendASCII(
            "codex.exe"));
  }
#else
  if (!home.empty()) {
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
#endif
  for (const base::FilePath& candidate : candidates) {
    if (IsRunnable(candidate)) {
      return candidate;
    }
  }
  return std::nullopt;
}

// A pipe whose `child` end is given to Codex and whose other end stays
// here: not inherited by other children Chromium starts.
bool MakePipe(bool child_reads, base::File* child, base::File* ours) {
#if BUILDFLAG(IS_WIN)
  SECURITY_ATTRIBUTES attributes = {sizeof(attributes), nullptr, TRUE};
  HANDLE read_end = nullptr;
  HANDLE write_end = nullptr;
  if (!CreatePipe(&read_end, &write_end, &attributes, 0)) {
    return false;
  }
  HANDLE our_end = child_reads ? write_end : read_end;
  SetHandleInformation(our_end, HANDLE_FLAG_INHERIT, 0);
  *child = base::File(child_reads ? read_end : write_end);
  *ours = base::File(our_end);
  return true;
#else
  // pipe2(O_CLOEXEC) where there is one (not macOS).
  int fds[2];
  if (pipe(fds) != 0) {
    return false;
  }
  for (int fd : fds) {
    if (fcntl(fd, F_SETFD, FD_CLOEXEC) != 0) {
      close(fds[0]);
      close(fds[1]);
      return false;
    }
  }
  *child = base::File(child_reads ? fds[0] : fds[1]);
  *ours = base::File(child_reads ? fds[1] : fds[0]);
  return true;
#endif
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
    base::File child_in, child_out, our_in, our_out;
    if (!MakePipe(/*child_reads=*/true, &child_in, &our_in) ||
        !MakePipe(/*child_reads=*/false, &child_out, &our_out)) {
      Closed("Couldn't start Codex (no pipes).");
      return;
    }
    // Its stderr goes to a log next to its working directory.
    base::File log(working_dir.AppendASCII("codex-app-server.log"),
                   base::File::FLAG_CREATE_ALWAYS | base::File::FLAG_WRITE);
    base::LaunchOptions options;
    options.current_directory = working_dir;
    base::CommandLine command(*codex);
#if BUILDFLAG(IS_WIN)
    // npm installs a .cmd script, which only cmd.exe runs.
    if (codex->MatchesExtension(FILE_PATH_LITERAL(".cmd")) ||
        codex->MatchesExtension(FILE_PATH_LITERAL(".bat"))) {
      base::FilePath system;
      base::PathService::Get(base::DIR_SYSTEM, &system);
      command = base::CommandLine(system.AppendASCII("cmd.exe"));
      command.AppendArg("/d");
      command.AppendArg("/c");
      command.AppendArgPath(*codex);
    }
    if (!log.IsValid()) {
      log = base::File(base::FilePath(L"NUL"),
                       base::File::FLAG_OPEN | base::File::FLAG_WRITE);
    }
    SetHandleInformation(log.GetPlatformFile(), HANDLE_FLAG_INHERIT,
                         HANDLE_FLAG_INHERIT);
    options.stdin_handle = child_in.GetPlatformFile();
    options.stdout_handle = child_out.GetPlatformFile();
    options.stderr_handle = log.GetPlatformFile();
    options.handles_to_inherit = {options.stdin_handle, options.stdout_handle,
                                  options.stderr_handle};
    options.start_hidden = true;
    // Closing the job ends codex and whatever it started (node, helpers).
    job_.Set(CreateJobObject(nullptr, nullptr));
    if (job_.is_valid() &&
        base::SetJobObjectLimitFlags(job_.get(),
                                     JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE)) {
      options.job_handle = job_.get();
    }
#else
    options.new_process_group = true;
    options.fds_to_remap.emplace_back(child_in.GetPlatformFile(),
                                      STDIN_FILENO);
    options.fds_to_remap.emplace_back(child_out.GetPlatformFile(),
                                      STDOUT_FILENO);
    if (log.IsValid()) {
      options.fds_to_remap.emplace_back(log.GetPlatformFile(), STDERR_FILENO);
    }
    // Codex finds its sign-in under $HOME.
    if (const base::FilePath home = HomeDir(); !home.empty()) {
      options.environment["HOME"] = home.value();
    }
#endif
    // Codex's own directory first: an npm install is a script that needs
    // the `node` next to it, which a desktop-launched browser's PATH (the
    // Dock's, on macOS) may not have.
    {
      base::NativeEnvironmentString path = codex->DirName().value();
      std::optional<std::string> inherited =
          base::Environment::Create()->GetVar("PATH");
#if BUILDFLAG(IS_WIN)
      path += L";" + base::FilePath::FromUTF8Unsafe(
                         inherited.value_or(std::string()))
                         .value();
#else
      path += ":" + (inherited && !inherited->empty()
                         ? *inherited
                         : std::string("/usr/local/bin:/usr/bin:/bin"));
#endif
      options.environment[FILE_PATH_LITERAL("PATH")] = path;
    }
    command.AppendArg("app-server");
    process_ = base::LaunchProcess(command, options);
    // The child has its ends now.
    child_in.Close();
    child_out.Close();
    if (!process_.IsValid()) {
      Closed("Couldn't start Codex.");
      return;
    }
    stdin_ = std::move(our_in);
    reader_->PostTask(FROM_HERE,
                      base::BindOnce(&Io::ReadLoop, this, std::move(our_out)));
    for (std::string& line : queued_) {
      WriteOnRunner(std::move(line));
    }
    queued_.clear();
  }

  // Blocks on the server's stdout until it closes.
  void ReadLoop(base::File out) {
    std::string buffer;
    uint8_t chunk[16384];
    while (true) {
      const std::optional<size_t> n =
          out.ReadAtCurrentPosNoBestEffort(base::span(chunk));
      if (!n || *n == 0) {
        break;
      }
      buffer.append(reinterpret_cast<const char*>(chunk), *n);
      size_t newline;
      while ((newline = buffer.find('\n')) != std::string::npos) {
        std::string line = buffer.substr(0, newline);
        buffer.erase(0, newline + 1);
        // Windows line endings.
        if (!line.empty() && line.back() == '\r') {
          line.pop_back();
        }
        if (!line.empty()) {
          ui_->PostTask(FROM_HERE, base::BindOnce(&CodexServer::OnLine, server_,
                                                  std::move(line)));
        }
      }
    }
    out.Close();
    Closed("The local Codex app server disconnected.");
  }

  void WriteOnRunner(std::string line) {
    if (!stdin_.IsValid()) {
      if (!process_.IsValid()) {
        queued_.push_back(std::move(line));
      }
      return;
    }
    line.push_back('\n');
    stdin_.WriteAtCurrentPos(base::as_byte_span(line));
  }

  void Kill() {
    // Codex exits when its input closes; give it a moment.
    stdin_.Close();
    if (!process_.IsValid()) {
      return;
    }
#if BUILDFLAG(IS_WIN)
    if (!process_.WaitForExitWithTimeout(base::Seconds(2), nullptr)) {
      process_.Terminate(/*exit_code=*/1, /*wait=*/false);
    }
    // Ends whatever it started too.
    job_.Close();
#else
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
#endif
  }

  scoped_refptr<base::SequencedTaskRunner> ui_;
  base::WeakPtr<CodexServer> server_;
  scoped_refptr<base::SequencedTaskRunner> runner_;
  scoped_refptr<base::SingleThreadTaskRunner> reader_;
  base::Process process_;
  base::File stdin_;
#if BUILDFLAG(IS_WIN)
  base::win::ScopedHandle job_;
#endif
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

#include "chrome/browser/ui/views/mori/mori_json_file.h"

#include <string>
#include <utility>

#include "base/files/file_util.h"
#include "base/files/important_file_writer.h"
#include "base/functional/bind.h"
#include "base/json/json_reader.h"
#include "base/json/json_writer.h"
#include "base/task/sequenced_task_runner.h"
#include "base/task/thread_pool.h"

namespace mori {

JsonFile::JsonFile(
    base::FilePath path,
    base::OnceCallback<void(std::optional<base::Value>)> on_loaded)
    : path_(std::move(path)),
      runner_(base::ThreadPool::CreateSequencedTaskRunner(
          {base::MayBlock(), base::TaskShutdownBehavior::BLOCK_SHUTDOWN})) {
  runner_->PostTaskAndReplyWithResult(
      FROM_HERE,
      base::BindOnce(
          [](base::FilePath path) {
            std::string json;
            base::ReadFileToString(path, &json);
            return base::JSONReader::Read(
                json, base::JSON_PARSE_CHROMIUM_EXTENSIONS);
          },
          path_),
      std::move(on_loaded));
}

JsonFile::~JsonFile() = default;

void JsonFile::Write(const base::Value& value) {
  std::string json;
  base::JSONWriter::WriteWithOptions(
      value, base::JSONWriter::OPTIONS_PRETTY_PRINT, &json);
  runner_->PostTask(FROM_HERE,
                    base::BindOnce(
                        [](base::FilePath path, std::string json) {
                          base::ImportantFileWriter::WriteFileAtomically(
                              path, json);
                        },
                        path_, std::move(json)));
}

}  // namespace mori

// A JSON file in the profile directory that Mori keeps its own data in
// (boosts, space themes, routes): read once on a background sequence, then
// rewritten atomically on every change.

#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_JSON_FILE_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_JSON_FILE_H_

#include <optional>

#include "base/files/file_path.h"
#include "base/functional/callback.h"
#include "base/memory/scoped_refptr.h"
#include "base/memory/weak_ptr.h"
#include "base/values.h"

namespace base {
class SequencedTaskRunner;
}

namespace mori {

class JsonFile {
 public:
  // `on_loaded` gets the file's value, or nothing if it is missing or not
  // JSON.
  JsonFile(base::FilePath path,
           base::OnceCallback<void(std::optional<base::Value>)> on_loaded);
  ~JsonFile();

  void Write(const base::Value& value);

 private:
  base::FilePath path_;
  scoped_refptr<base::SequencedTaskRunner> runner_;
  base::WeakPtrFactory<JsonFile> weak_factory_{this};
};

}  // namespace mori

#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_JSON_FILE_H_

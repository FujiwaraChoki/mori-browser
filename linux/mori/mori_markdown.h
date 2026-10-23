// Lightweight native rendering model for assistant Markdown. Text and code
// remain text; the view decides which link schemes may be opened.
#ifndef CHROME_BROWSER_UI_VIEWS_MORI_MORI_MARKDOWN_H_
#define CHROME_BROWSER_UI_VIEWS_MORI_MORI_MARKDOWN_H_

#include <cctype>
#include <string>
#include <string_view>
#include <utility>
#include <vector>

namespace mori::markdown {

struct Span {
  std::string text;
  bool bold = false;
  bool italic = false;
  bool code = false;
  std::string link;
};

struct Block {
  enum Kind { kParagraph, kHeading, kCode, kQuote, kRule };
  Kind kind = kParagraph;
  int level = 0;
  std::string prefix;
  std::vector<Span> spans;
};

inline void Append(std::vector<Span>& out, std::string_view text,
                   const Span& style) {
  if (text.empty()) {
    return;
  }
  if (!out.empty() && out.back().bold == style.bold &&
      out.back().italic == style.italic && out.back().code == style.code &&
      out.back().link == style.link) {
    out.back().text.append(text);
  } else {
    Span span = style;
    span.text = text;
    out.push_back(std::move(span));
  }
}

inline std::string_view Trim(std::string_view text);

inline std::vector<Span> Inline(std::string_view text, Span style = {},
                                int depth = 0) {
  std::vector<Span> out;
  if (depth > 12) {
    Append(out, text, style);
    return out;
  }
  for (size_t i = 0; i < text.size();) {
    if (text[i] == '\\' && i + 1 < text.size() &&
        std::ispunct(static_cast<unsigned char>(text[i + 1]))) {
      Append(out, text.substr(i + 1, 1), style);
      i += 2;
      continue;
    }
    if (text[i] == char(0x60)) {
      size_t count = 1;
      while (i + count < text.size() && text[i + count] == char(0x60)) {
        ++count;
      }
      const std::string marker(count, char(0x60));
      const size_t end = text.find(marker, i + count);
      if (end != std::string_view::npos) {
        Span code = style;
        code.code = true;
        Append(out, text.substr(i + count, end - i - count), code);
        i = end + count;
        continue;
      }
    }
    if (text[i] == '[') {
      const size_t label_end = text.find("](", i + 1);
      if (label_end != std::string_view::npos) {
        size_t end = label_end + 2;
        int nesting = 1;
        for (; end < text.size(); ++end) {
          if (text[end] == '\\' && end + 1 < text.size()) {
            ++end;
          } else if (text[end] == '(') {
            ++nesting;
          } else if (text[end] == ')' && --nesting == 0) {
            break;
          }
        }
        if (end < text.size()) {
          Span link = style;
          auto url = Trim(text.substr(label_end + 2, end - label_end - 2));
          // Optional Markdown link titles aren't part of the destination.
          if (url.starts_with("<") && url.find('>') != std::string_view::npos) {
            url = url.substr(1, url.find('>') - 1);
          } else {
            url = url.substr(0, url.find_first_of(" \t\r\n"));
          }
          for (size_t j = 0; j < url.size(); ++j) {
            if (url[j] == '\\' && j + 1 < url.size() &&
                std::ispunct(static_cast<unsigned char>(url[j + 1]))) {
              ++j;
            }
            link.link += url[j];
          }
          auto children = Inline(text.substr(i + 1, label_end - i - 1),
                                 link, depth + 1);
          out.insert(out.end(), children.begin(), children.end());
          i = end + 1;
          continue;
        }
      }
    }
    if (text[i] == '<') {
      const size_t end = text.find('>', i + 1);
      const auto url = text.substr(i + 1, end - i - 1);
      if (end != std::string_view::npos &&
          (url.starts_with("https://") || url.starts_with("http://"))) {
        Span link = style;
        link.link = url;
        Append(out, url, link);
        i = end + 1;
        continue;
      }
    }
    bool handled = false;
    for (const std::string_view marker :
         {"***", "___", "**", "__", "*", "_"}) {
      if (!text.substr(i).starts_with(marker)) {
        continue;
      }
      // Preserve underscores in identifiers and file names.
      if (marker[0] == '_' && i > 0 &&
          std::isalnum(static_cast<unsigned char>(text[i - 1]))) {
        continue;
      }
      size_t end = text.find(marker, i + marker.size());
      while (end != std::string_view::npos) {
        size_t run_end = end + marker.size();
        while (run_end < text.size() && text[run_end] == marker[0]) {
          ++run_end;
        }
        // A strong delimiter inside emphasis isn't its closing delimiter.
        // At a combined closing run, the outer delimiter closes last.
        if ((end > 0 && text[end - 1] == '\\') ||
            (marker.size() == 1 && (run_end - end) % 2 == 0)) {
          end = text.find(marker, run_end);
          continue;
        }
        end = run_end - marker.size();
        break;
      }
      if (end == std::string_view::npos || end == i + marker.size()) {
        continue;
      }
      Span emphasis = style;
      emphasis.bold = emphasis.bold || marker.size() >= 2;
      emphasis.italic = emphasis.italic || marker.size() != 2;
      auto children = Inline(
          text.substr(i + marker.size(), end - i - marker.size()),
          emphasis, depth + 1);
      out.insert(out.end(), children.begin(), children.end());
      i = end + marker.size();
      handled = true;
      break;
    }
    if (!handled) {
      Append(out, text.substr(i, 1), style);
      ++i;
    }
  }
  return out;
}

inline std::string_view Trim(std::string_view text) {
  const size_t first = text.find_first_not_of(" \t\r");
  if (first == std::string_view::npos) {
    return {};
  }
  return text.substr(first, text.find_last_not_of(" \t\r") - first + 1);
}

inline std::vector<Block> Parse(std::string_view markdown) {
  std::vector<Block> blocks;
  std::string paragraph;
  std::string code;
  std::string fence;
  bool has_code_line = false;
  bool hard_break = false;
  auto flush = [&] {
    if (!paragraph.empty()) {
      blocks.push_back({Block::kParagraph, 0, {}, Inline(paragraph)});
      paragraph.clear();
    }
    hard_break = false;
  };
  while (!markdown.empty()) {
    const size_t end = markdown.find('\n');
    std::string_view line = markdown.substr(0, end);
    if (line.ends_with('\r')) {
      line.remove_suffix(1);
    }
    markdown = end == std::string_view::npos ? std::string_view()
                                            : markdown.substr(end + 1);
    const auto trimmed = Trim(line);
    if (!fence.empty()) {
      const size_t fence_end = trimmed.find_first_not_of(fence[0]);
      if (trimmed.starts_with(fence) &&
          (fence_end == std::string_view::npos ||
           Trim(trimmed.substr(fence_end)).empty())) {
        blocks.push_back({Block::kCode, 0, {},
                          {{code, false, false, false, {}}}});
        code.clear();
        fence.clear();
        has_code_line = false;
      } else {
        if (has_code_line) {
          code += '\n';
        }
        code += line;
        has_code_line = true;
      }
      continue;
    }
    if (trimmed.size() >= 3 &&
        (trimmed[0] == char(0x60) || trimmed[0] == '~') &&
        trimmed[1] == trimmed[0] && trimmed[2] == trimmed[0]) {
      flush();
      const size_t count = trimmed.find_first_not_of(trimmed[0]);
      fence = trimmed.substr(0, count);
      continue;
    }
    if (trimmed.empty()) {
      flush();
      continue;
    }
    if (!paragraph.empty() && trimmed.size() >= 3 &&
        (trimmed[0] == '=' || trimmed[0] == '-') &&
        trimmed.find_first_not_of(trimmed[0]) == std::string_view::npos) {
      blocks.push_back({Block::kHeading, trimmed[0] == '=' ? 1 : 2, {},
                        Inline(paragraph)});
      paragraph.clear();
      hard_break = false;
      continue;
    }
    if (trimmed == "---" || trimmed == "***" || trimmed == "___") {
      flush();
      blocks.push_back({Block::kRule, 0, {}, {}});
      continue;
    }
    size_t heading = 0;
    while (heading < trimmed.size() && trimmed[heading] == '#') {
      ++heading;
    }
    if (heading > 0 && heading <= 6 && heading < trimmed.size() &&
        trimmed[heading] == ' ') {
      flush();
      blocks.push_back({Block::kHeading, static_cast<int>(heading), {},
                        Inline(Trim(trimmed.substr(heading + 1)))});
      continue;
    }
    if (trimmed.starts_with(">")) {
      flush();
      blocks.push_back({Block::kQuote, 0, {},
                        Inline(Trim(trimmed.substr(1)))});
      continue;
    }
    size_t marker = 0;
    std::string prefix;
    if (trimmed.starts_with("- ") || trimmed.starts_with("* ") ||
        trimmed.starts_with("+ ")) {
      marker = 2;
      prefix = "• ";
    } else {
      while (marker < trimmed.size() && trimmed[marker] >= '0' &&
             trimmed[marker] <= '9') {
        ++marker;
      }
      if (marker > 0 && marker + 1 < trimmed.size() &&
          (trimmed[marker] == '.' || trimmed[marker] == ')') &&
          trimmed[marker + 1] == ' ') {
        prefix = std::string(trimmed.substr(0, marker)) + ". ";
        marker += 2;
      } else {
        marker = 0;
      }
    }
    if (marker) {
      flush();
      prefix.insert(0, line.find_first_not_of(" \t"), ' ');
      blocks.push_back({Block::kParagraph, 0, prefix,
                        Inline(trimmed.substr(marker))});
      continue;
    }
    if (!paragraph.empty()) {
      paragraph += hard_break ? '\n' : ' ';
    }
    paragraph += trimmed;
    hard_break = line.ends_with("  ");
  }
  flush();
  if (!fence.empty()) {
    blocks.push_back({Block::kCode, 0, {},
                      {{code, false, false, false, {}}}});
  }
  return blocks;
}

}  // namespace mori::markdown
#endif  // CHROME_BROWSER_UI_VIEWS_MORI_MORI_MARKDOWN_H_

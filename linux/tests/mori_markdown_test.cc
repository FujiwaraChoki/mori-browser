// Standalone parser regressions: c++ -std=c++20 mori_markdown_test.cc -o test
#include "../mori/mori_markdown.h"

#include <cassert>
#include <iostream>

using mori::markdown::Block;
using mori::markdown::Inline;
using mori::markdown::Parse;

std::string Text(const std::vector<mori::markdown::Span>& spans) {
  std::string text;
  for (const auto& span : spans) {
    text += span.text;
  }
  return text;
}

int main() {
  auto spans = Inline("Opened [GitHub](https://github.com/) in a new tab.");
  assert(Text(spans) == "Opened GitHub in a new tab.");
  assert(spans.size() == 3);
  assert(spans[1].text == "GitHub");
  assert(spans[1].link == "https://github.com/");

  spans = Inline("**bold *and italic***, plain");
  assert(Text(spans) == "bold and italic, plain");
  assert(spans[0].bold && !spans[0].italic);
  assert(spans[1].bold && spans[1].italic);
  spans = Inline("*italic **with bold***");
  assert(Text(spans) == "italic with bold");
  assert(spans[0].italic && !spans[0].bold);
  assert(spans[1].italic && spans[1].bold);

  spans = Inline("[**Schöne Grüße 🌍**](https://example.com/a_(b))");
  assert(spans.size() == 1 && spans[0].bold);
  assert(spans[0].text == "Schöne Grüße 🌍");
  assert(spans[0].link == "https://example.com/a_(b)");
  spans = Inline("[GitHub](https://github.com/ \"Site title\")");
  assert(spans[0].link == "https://github.com/");
  spans = Inline("[Page](<https://example.com/> 'Title')");
  assert(spans[0].link == "https://example.com/");

  spans = Inline("a snake_case_name and \\*literal\\*");
  assert(Text(spans) == "a snake_case_name and *literal*");
  assert(spans.size() == 1);

  // Never interpret inline code as formatting or a clickable link.
  const std::string ticks(1, char(0x60));
  spans = Inline(ticks + "**raw** [x](https://example.com)" + ticks);
  assert(spans.size() == 1 && spans[0].code && spans[0].link.empty());
  assert(spans[0].text == "**raw** [x](https://example.com)");

  spans = Inline("Opened [GitHub](https://github.");
  assert(Text(spans) == "Opened [GitHub](https://github.");
  spans = Inline("<script>alert(1)</script>");
  assert(Text(spans) == "<script>alert(1)</script>");
  assert(spans[0].link.empty());
  spans = Inline("<https://example.com>");
  assert(spans[0].text == spans[0].link);

  auto blocks = Parse("# Heading\n\nFirst line\nsecond line.\n\n"
                      "- First\n  - **Nested**\n2. Ordered\n> Quote\n---");
  assert(blocks.size() == 7);
  assert(blocks[0].kind == Block::kHeading && blocks[0].level == 1);
  assert(Text(blocks[1].spans) == "First line second line.");
  assert(blocks[2].prefix == "• ");
  assert(blocks[3].prefix == "  • " && blocks[3].spans[0].bold);
  assert(blocks[4].prefix == "2. ");
  assert(blocks[5].kind == Block::kQuote);
  assert(blocks[6].kind == Block::kRule);
  blocks = Parse("Title\r\n===\r\n\r\nFirst  \r\nNext");
  assert(blocks[0].kind == Block::kHeading && blocks[0].level == 1);
  assert(Text(blocks[1].spans) == "First\nNext");

  const std::string fence(3, char(0x60));
  blocks = Parse(fence + "cpp\n\n  **literal**\n" + fence + "\nAfter.");
  assert(blocks.size() == 2 && blocks[0].kind == Block::kCode);
  assert(blocks[0].spans[0].text == "\n  **literal**");
  assert(Text(blocks[1].spans) == "After.");
  blocks = Parse(fence + "\nstreaming code");
  assert(blocks.size() == 1 && blocks[0].spans[0].text == "streaming code");
  assert(Parse("").empty());
  assert(Parse(" \n\t\r\n").empty());
  // Streamed messages may end in the middle of any delimiter or URL.
  const std::string streamed = "# Hi\n\n[**Grüße 🌍**](https://example.com/a_(b))\n" +
                               fence + "cpp\n**literal**\n" + fence;
  for (size_t end = 0; end <= streamed.size(); ++end) {
    Parse(std::string_view(streamed).substr(0, end));
  }
  std::cout << "Markdown parser regressions passed.\n";
}

import Foundation
import WebKit

struct ReaderArticle: Identifiable {
    struct Block: Identifiable {
        let id = UUID()
        var tag: String
        var text: String
    }

    let id = UUID()
    var title: String
    var byline: String
    var blocks: [Block]

    static func decode(_ json: String) -> ReaderArticle? {
        guard let data = json.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        let title = (object["title"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let byline = (object["byline"] as? String)?.trimmingCharacters(in: .whitespacesAndNewlines) ?? ""
        let raw = object["blocks"] as? [[String: Any]] ?? []
        let blocks = raw.compactMap { item -> Block? in
            guard let text = item["text"] as? String else { return nil }
            let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
            guard trimmed.count >= 2 else { return nil }
            return Block(tag: (item["tag"] as? String) ?? "p", text: trimmed)
        }
        guard blocks.count >= 2 else { return nil }
        return ReaderArticle(title: title.isEmpty ? "Reader" : title, byline: byline, blocks: blocks)
    }
}

enum ReaderScript {
    static let source = """
    (function() {
      function text(node) { return (node.innerText || '').replace(/[ \\t]+\\n/g, '\\n').trim(); }
      const drop = 'script,style,noscript,iframe,svg,canvas,form,nav,footer,header,aside,[role="navigation"],[role="banner"],[role="contentinfo"]';
      function score(el) {
        let total = 0;
        el.querySelectorAll('p').forEach(function(p) {
          const value = text(p);
          if (value.length > 40) total += value.length;
        });
        return total;
      }
      let best = document.querySelector('article') || document.querySelector('main');
      if (!best || score(best) < 200) {
        let top = 0;
        document.querySelectorAll('article,main,section,div').forEach(function(el) {
          const value = score(el);
          if (value > top && (el.innerText || '').length < 200000) { top = value; best = el; }
        });
      }
      if (!best) best = document.body;
      const clone = best.cloneNode(true);
      clone.querySelectorAll(drop).forEach(function(node) { node.remove(); });
      const blocks = [];
      clone.querySelectorAll('h1,h2,h3,p,li,blockquote,pre').forEach(function(el) {
        const value = text(el);
        if (value.length < 2) return;
        blocks.push({ tag: el.tagName.toLowerCase(), text: value });
      });
      const heading = document.querySelector('h1');
      const title = (heading && text(heading)) || document.title || '';
      const author = document.querySelector('meta[name="author"]');
      return JSON.stringify({ title: title, byline: author ? (author.content || '') : '', blocks: blocks.slice(0, 400) });
    })()
    """
}

extension WebSession {
    func extractArticle() async -> ReaderArticle? {
        guard let value = try? await webView.evaluateJavaScript(ReaderScript.source) as? String else { return nil }
        return ReaderArticle.decode(value)
    }
}

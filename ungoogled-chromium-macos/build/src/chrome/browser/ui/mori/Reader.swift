import SwiftUI

extension BrowserStore {
    /// Toggle Reader Mode on the active tab.
    func toggleReader() {
        selectedTab?.toggleReader()
    }

    var canUseReader: Bool {
        guard let url = selectedTab?.urlString else { return false }
        return url.hasPrefix("http://") || url.hasPrefix("https://")
    }
}

/// Page scripts for the distraction-free Reader view. Disabling Reader just
/// reloads the original page, so only an "enable" script is needed.
enum ReaderScripts {
    /// Extract the main article and replace the document with a clean, themed
    /// reading view. Returns true when an article was found and rendered.
    static let enable = """
    (() => {
      try {
        if (window.__moriReaderOn) return true;
        const pick = () => {
          const direct = document.querySelector('article')
            || document.querySelector('[role=main]')
            || document.querySelector('main');
          if (direct && (direct.innerText || '').length > 250) return direct;
          let best = null, bestLen = 0;
          document.querySelectorAll('div,section,article').forEach((el) => {
            const ps = el.querySelectorAll('p');
            if (ps.length < 3) return;
            let len = 0;
            ps.forEach((p) => { len += (p.innerText || '').length; });
            if (len > bestLen) { bestLen = len; best = el; }
          });
          return best;
        };
        const node = pick();
        if (!node || (node.innerText || '').length < 250) return false;
        const title = (document.querySelector('h1') && document.querySelector('h1').innerText)
          || document.title || '';
        const dark = matchMedia('(prefers-color-scheme: dark)').matches;
        const article = node.cloneNode(true);
        article.querySelectorAll(
          'script,style,iframe,noscript,aside,nav,footer,header,form,button,svg,[role=navigation],[aria-hidden=true]'
        ).forEach((e) => e.remove());
        const bg = dark ? '#1b1b1d' : '#faf9f6';
        const fg = dark ? '#e7e7e7' : '#1a1a1a';
        const muted = dark ? '#9a9a9e' : '#6b6b6b';
        const link = dark ? '#8ab4f8' : '#1a64d6';
        document.documentElement.innerHTML = '<head><meta charset="utf-8"></head><body></body>';
        const style = document.createElement('style');
        style.textContent = `
          html,body{margin:0;background:${bg};}
          .mori-reader{max-width:720px;margin:0 auto;padding:72px 24px 140px;
            font:19px/1.72 ui-serif,Georgia,'Times New Roman',serif;color:${fg};
            -webkit-font-smoothing:antialiased;}
          .mori-reader h1{font:600 34px/1.22 -apple-system,BlinkMacSystemFont,sans-serif;
            margin:0 0 28px;letter-spacing:-0.01em;}
          .mori-reader h2,.mori-reader h3,.mori-reader h4{
            font-family:-apple-system,BlinkMacSystemFont,sans-serif;line-height:1.3;margin:1.7em 0 .5em;}
          .mori-reader p{margin:0 0 1.15em;}
          .mori-reader img,.mori-reader figure img{max-width:100%;height:auto;border-radius:10px;margin:1.2em 0;}
          .mori-reader figure{margin:1.4em 0;}
          .mori-reader figcaption{color:${muted};font-size:14px;text-align:center;margin-top:.5em;}
          .mori-reader a{color:${link};text-decoration:none;}
          .mori-reader a:hover{text-decoration:underline;}
          .mori-reader blockquote{margin:1.4em 0;padding:.2em 0 .2em 1.1em;
            border-left:3px solid ${muted};color:${muted};}
          .mori-reader pre,.mori-reader code{font-family:ui-monospace,SFMono-Regular,monospace;font-size:15px;}
          .mori-reader pre{background:${dark ? '#111' : '#f0ede6'};padding:14px;border-radius:8px;overflow:auto;}
          ::selection{background:${dark ? '#3a3a3c' : '#dcd6c8'};}
        `;
        document.head.appendChild(style);
        const wrap = document.createElement('div');
        wrap.className = 'mori-reader';
        const h = document.createElement('h1');
        h.textContent = title;
        wrap.appendChild(h);
        wrap.appendChild(article);
        document.body.appendChild(wrap);
        window.scrollTo(0, 0);
        window.__moriReaderOn = true;
        return true;
      } catch (e) { return false; }
    })()
    """
}

/// Omnibox toggle for Reader Mode, shown on web pages.
struct ReaderButton: View {
    @ObservedObject var store: BrowserStore
    @ObservedObject var tab: BrowserTab
    @Environment(\.palette) private var p

    var body: some View {
        if tab.urlString.hasPrefix("http") {
            Button { tab.toggleReader() } label: {
                Icon(name: tab.readerActive ? "doc.plaintext.fill" : "doc.plaintext", size: 14)
                    .foregroundStyle(tab.readerActive ? p.accent.color : p.mutedForeground.color)
                    .frame(width: 22, height: 22)
                    .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help(tab.readerActive ? "Exit Reader" : "Reader View")
        }
    }
}

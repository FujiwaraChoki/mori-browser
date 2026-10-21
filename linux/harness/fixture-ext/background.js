// Test only. Builds a sidebar like an Arc user's: two pinned tabs
// (favorites) and a few tabs.
chrome.runtime.onInstalled.addListener(async () => {
  const open = (url) => chrome.tabs.create({ url, active: false });
  const pinned = [await open("https://github.com"), await open("https://en.wikipedia.org")];
  for (const tab of pinned) await chrome.tabs.update(tab.id, { pinned: true });
  for (const url of ["https://developer.mozilla.org", "https://news.ycombinator.com",
                     "https://www.youtube.com", "https://www.reddit.com"]) {
    await open(url);
  }
});

// Ctrl+Shift+U: open a tab as an extension or a link would.
chrome.commands.onCommand.addListener((command) => {
  if (command === "open-tab") chrome.tabs.create({ url: "https://example.org" });
});

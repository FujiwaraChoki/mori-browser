// Test only. Builds a sidebar like an Arc user's: two pinned tabs
// (favorites), a "Work" and a "Personal" group, and a loose tab.
chrome.runtime.onInstalled.addListener(async () => {
  const open = (url) => chrome.tabs.create({ url, active: false });
  const pinned = [await open("https://github.com"), await open("https://en.wikipedia.org")];
  for (const tab of pinned) await chrome.tabs.update(tab.id, { pinned: true });
  const work = [await open("https://developer.mozilla.org"), await open("https://news.ycombinator.com")];
  const workGroup = await chrome.tabs.group({ tabIds: work.map((t) => t.id) });
  await chrome.tabGroups.update(workGroup, { title: "Work", color: "blue" });
  const personal = [await open("https://www.youtube.com"), await open("https://www.reddit.com")];
  const personalGroup = await chrome.tabs.group({ tabIds: personal.map((t) => t.id) });
  await chrome.tabGroups.update(personalGroup, { title: "Personal", color: "pink" });
});

// Ctrl+Shift+U: open a tab as an extension or a link would, to check that it
// joins the space on screen.
chrome.commands.onCommand.addListener((command) => {
  if (command === "open-tab") chrome.tabs.create({ url: "https://example.org" });
});

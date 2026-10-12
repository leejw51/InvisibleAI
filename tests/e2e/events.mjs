// Observe the real HTTP EventSource transport through Chromium, without replacing app code.
export async function observeEvents(page, callback) {
  const session = await page.context().newCDPSession(page);
  const urls = new Map();
  await session.send('Network.enable');
  session.on('Network.requestWillBeSent', e => urls.set(e.requestId, e.request.url));
  session.on('Network.eventSourceMessageReceived', e => {
    const url = urls.get(e.requestId) || '';
    const memoId = url.match(/\/api\/memos\/([^/]+)\/events/)?.[1];
    if (memoId) callback(JSON.parse(e.data), memoId);
  });
  return session;
}

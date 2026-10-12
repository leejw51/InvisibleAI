// The whole user journey on one backend: upload anything in chat, get an answer,
// see the session and cost on Autonomous AI, find it organized in the Wiki with
// its media and usage, read the original, and come back to the same node.
import {test,expect} from '@playwright/test';
import fs from 'node:fs';

test('a user uploads files in chat and finds everything organized, costed, and navigable',async({page,request},info)=>{
  await request.patch('/api/assistance/preferences',{data:{wiki_enabled:true}});
  // Browser projects share one demo backend: a project-specific tag keeps each
  // run's Wiki node separate, so the counts below stay exact.
  const tag='harbour'+info.project.name.replace(/[^a-z]/g,'');
  // 1. Chat: attach markdown, HTML, a picture and a short video, then ask.
  await page.goto('/chat');
  await expect(page.getByRole('heading',{name:'Keep the thought going.'})).toBeVisible();
  const rooms=page.getByRole('combobox',{name:'Chat room',exact:true});
  const before=await rooms.inputValue();
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  await expect(rooms).not.toHaveValue(before);
  await page.getByLabel('Attach to chat',{exact:true}).setInputFiles([
    {name:'brief.md',mimeType:'text/markdown',buffer:Buffer.from('# Brief\nShip the harbour page by Friday.')},
    {name:'page.html',mimeType:'text/html',buffer:Buffer.from('<html><head><title>Harbour</title><style>p{}</style></head><body><h1>Harbour plan</h1><p>Open at dawn &amp; close at dusk.</p></body></html>')},
    {name:'harbour.png',mimeType:'image/png',buffer:fs.readFileSync('frontend/assets/harbour.png')},
    {name:'clip.mp4',mimeType:'video/mp4',buffer:fs.readFileSync('tests/e2e/fixtures/clip.mp4')},
  ]);
  await expect(page.locator('.chat-attachment')).toHaveCount(4);
  await expect(page.locator('.attachment-details').nth(0)).toContainText('Text context');
  await expect(page.locator('.attachment-details').nth(1)).toContainText('Text context');
  await page.getByRole('textbox',{name:'Chat message',exact:true}).fill(`Summarize the harbour plan from these files. #${tag}`);
  await page.getByRole('button',{name:'Send message',exact:true}).click();
  await expect(page.locator('.from-ai .chat-message-meta')).toContainText('Cloud LLM');
  await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
  await expect(page.locator('.from-ai [data-testid="chat-llm-log"]')).toContainText('Answered by Cloud AI');
  // The HTML became readable text for the model: no tags in the saved input.
  const memos=await (await request.get('/api/memos')).json();
  const memo=(memos.memos||memos).find(m=>m.input.includes('#'+tag));
  expect(memo.input).toContain('Harbour plan');
  expect(memo.input).toContain('Open at dawn & close at dusk.');
  expect(memo.input).not.toContain('<h1>');
  // 2. Autonomous AI: the cost banner and mode cards are there for this account.
  await page.goto('/autonomous');
  await expect(page.getByTestId('cost-banner')).toBeVisible();
  await expect(page.getByTestId('cost-today')).toContainText('cloud calls');
  await expect(page.getByRole('group',{name:'AI processing mode'}).getByRole('button')).toHaveCount(5);
  // 3. Wiki: the message is one card with its picture, the video frame, the
  // cloud usage and the room it was said in.
  await page.goto('/wiki');
  await page.getByRole('searchbox',{name:'Search Wiki'}).fill('#'+tag);
  const card=page.locator('.wiki-card');
  await expect(card).toHaveCount(1);
  await expect(card.locator('.wiki-tag').filter({hasText:'#'+tag})).toBeVisible();
  await expect(card.locator('.wiki-card-thumb img')).toHaveAttribute('src',/^\/api\/files\/[^/]+\/thumbnail$/);
  await expect(card.locator('.wiki-cloud')).toContainText('cloud');
  await expect(card.getByTestId('wiki-room')).toHaveAttribute('href',/^\/chat\?room=/);
  // The tag's document holds the readable text, both media files and the cost.
  await card.locator('.wiki-tag').filter({hasText:'#'+tag}).click();
  await expect(page).toHaveURL(new RegExp('/wiki/tag/'+tag+'$'));
  await page.getByTestId('wiki-open-document').click();
  await expect(page).toHaveURL(/\/wiki\/[a-f0-9]{64}$/);
  const nodeUrl=page.url();
  const article=page.locator('.wiki-article');
  await expect(article).toContainText('Harbour plan');
  await expect(article.locator('img[src^="/api/files/"][src$="/image"]')).toHaveCount(1);
  await expect(article.locator('img[src$="/thumbnail"]')).toHaveCount(1);
  await expect(article.getByRole('link',{name:/brief\.md/})).toBeVisible();
  await expect(page.getByTestId('wiki-usage')).toContainText('AI requests');
  // 4. Read the original in place, see its cost, and come back to the same document.
  await page.getByRole('link',{name:'Open original source'}).first().click();
  await expect(page).toHaveURL(/\/wiki\/memo\/[a-f\d-]{36}$/);
  const source=page.getByTestId('wiki-source');
  await expect(source.locator('.wiki-cloud')).toContainText('cloud');
  await expect(source.locator('.wiki-source-answer .markdown')).not.toBeEmpty();
  await page.goBack();
  await expect(page).toHaveURL(nodeUrl);
  await expect(article).toBeVisible();
  // 5. The Wiki menu reopens the remembered tag; showing everything forgets it.
  await page.goto('/wiki');
  await expect(page).toHaveURL(new RegExp('/wiki/tag/'+tag+'$'));
  await page.getByTestId('wiki-clear-tag').click();
  await expect(page).toHaveURL(/\/wiki$/);
  await page.reload();
  await expect(page.locator('.wiki-card').first()).toBeVisible();
  await expect(page).toHaveURL(/\/wiki$/);
  // 6. The map shows the tag too and filters by it.
  await page.getByTestId('wiki-graph-toggle').click();
  await expect(page.getByTestId('wiki-graph')).toHaveAttribute('data-renderer',/three|unavailable/);
  const entry=page.locator('wiki-graph .wiki-graph-label.is-tag, .wiki-graph-fallback li button').filter({hasText:'#'+tag}).first();
  await expect(entry).toBeVisible();
  await entry.evaluate(el=>el.click());
  await expect(page.locator('.wiki-active-tag')).toHaveText('#'+tag);
  await expect(page).toHaveURL(/\/wiki\/map$/);
  expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBeTruthy();
  await page.screenshot({path:`test-results/ux-flow-${info.project.name}.png`,fullPage:true});
});

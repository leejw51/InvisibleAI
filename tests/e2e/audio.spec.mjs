import {test, expect} from '@playwright/test';
import fs from 'node:fs';

const translations = JSON.parse(fs.readFileSync('crates/frontend/src/locales.json', 'utf8'));

// Demo speech exercises real room sync without a microphone or cloud calls.
test.beforeEach(async ({request}) => {
  expect((await request.delete('/api/audio')).ok()).toBe(true);
  expect((await request.post('/api/audio/room', {data: {fresh: true}})).ok()).toBe(true);
});
test.afterEach(async ({request}) => {
  expect((await request.delete('/api/audio')).ok()).toBe(true);
});

test('audio chat syncs heard speech, Solve answers and a whole-session Note', async ({page, browser, request}, info) => {
  const other = await browser.newPage();
  try {
    await page.goto('/audio');
    await other.goto('/audio');
    await expect(page.getByRole('heading', {name: /Your Mac takes the notes/})).toBeVisible();
    await expect(page.getByTestId('audio-stage')).toHaveText('Not listening');
    await expect(page.getByTestId('audio-solve')).toBeDisabled();
    await expect(page.getByTestId('audio-note')).toBeDisabled();

    await page.getByTestId('audio-start').click();
    await expect(page.getByTestId('audio-stage')).toHaveText('Listening…');
    const heard = page.locator('[data-kind="heard"]');
    await expect(heard).toHaveCount(1);
    await expect(heard.first()).toContainText('What is seven plus five?');
    await expect(heard.first()).toContainText('simple arithmetic');
    await expect(other.locator('[data-kind="heard"]')).toHaveText(await heard.allTextContents());
    await expect(page.getByTestId('audio-solve')).toContainText('2 new');
    await page.screenshot({path: info.outputPath('audio-listening.png'), fullPage: true});

    await page.getByTestId('audio-solve').click();
    const answers = page.getByTestId('audio-answer');
    await expect(answers).toHaveCount(1);
    await expect(answers.first()).toHaveAttribute('data-status', 'complete');
    await expect(answers.first().locator('.answer-content')).not.toBeEmpty();
    await expect(page.locator('[data-kind="request"]')).toHaveText(/You pressed Solve/);
    // Speech after Solve becomes a new Heard message below the answer.
    await expect(heard).toHaveCount(2);
    await expect(heard.last()).toContainText('multiplication');
    await expect(heard.last()).not.toContainText('simple arithmetic');
    expect(await page.getByTestId('audio-notes').locator('> li').evaluateAll(rows => rows.map(row => row.dataset.kind)))
      .toEqual(['heard', 'request', 'answer', 'heard']);
    await expect(page.getByTestId('audio-solve')).toContainText('2 new');
    await expect(other.getByTestId('audio-notes')).toHaveText(await page.getByTestId('audio-notes').textContent());

    await page.getByTestId('audio-stop').click();
    await expect(page.getByTestId('audio-stage')).toHaveText('Not listening');
    await expect(other.getByTestId('audio-stage')).toHaveText('Not listening');
    // Saved speech remains available to Solve after capture stops.
    await page.getByTestId('audio-solve').click();
    await expect(answers).toHaveCount(2);
    await expect(answers.last()).toHaveAttribute('data-status', 'complete');
    await expect(page.getByTestId('audio-solve')).toHaveText('Solve');
    await page.getByTestId('audio-note').click();
    await expect(answers).toHaveCount(3);
    await expect(answers.last()).toHaveAttribute('data-status', 'complete');
    await expect(answers.last().locator('strong')).toHaveText('Note');
    await expect(answers.last().locator('.answer-content')).not.toBeEmpty();
    await expect(other.getByTestId('audio-answer')).toHaveCount(3);
    await page.screenshot({path: info.outputPath('audio-solved.png'), fullPage: true});

    const status = await (await request.get('/api/audio')).json();
    const memo = await (await request.get(`/api/memos/${status.solve_memo}`)).json();
    expect(memo.kind).toBe('note');
    expect(memo.input).toBe('Note');
    await page.reload();
    await expect(page.getByTestId('audio-answer')).toHaveCount(3);
    await expect(heard).toHaveCount(2);
    await page.getByRole('button', {name: 'Open in Ask anything'}).click();
    await expect(page).toHaveURL(/\/chat$/);
    await expect(page.locator('.chat-user-text').filter({hasText: 'simple arithmetic'})).toBeVisible();
    await expect(page.locator('.chat-user-text').filter({hasText: /^Solve$/})).toHaveCount(2);
    await expect(page.locator('.chat-user-text').filter({hasText: /^Note$/})).toBeVisible();
  } finally {
    await other.close();
  }
});

test('New chat keeps listening, earlier rooms reopen and clearing stops capture', async ({page, browser, request}) => {
  const other = await browser.newPage();
  try {
    await page.goto('/audio');
    await other.goto('/audio');
    await page.getByTestId('audio-start').click();
    await expect(page.locator('[data-kind="heard"]')).toContainText('simple arithmetic');
    const first = (await (await request.get('/api/audio')).json()).room_id;
    await page.getByTestId('audio-new-chat').click();
    await expect(page.locator('[data-kind="heard"]')).toHaveCount(0);
    const second = (await (await request.get('/api/audio')).json()).room_id;
    expect(second).not.toBe(first);
    await expect(page.getByTestId('audio-room')).toHaveValue(second);
    await expect(other.getByTestId('audio-room')).toHaveValue(second);
    await expect(page.getByTestId('audio-stage')).toHaveText('Listening…');
    await expect(page.locator('[data-kind="heard"]')).toContainText('multiplication');
    await expect(page.locator('[data-kind="heard"]')).not.toContainText('simple arithmetic');

    await page.getByTestId('audio-room').selectOption(first);
    await expect(page.locator('[data-kind="heard"]')).toContainText('simple arithmetic');
    await expect(other.getByTestId('audio-room')).toHaveValue(first);
    await expect(other.locator('[data-kind="heard"]')).toContainText('simple arithmetic');
    // Erasure from another device stops capture and clears both views.
    expect((await request.post(`/api/rooms/${first}/clear`)).ok()).toBe(true);
    await expect(page.getByTestId('audio-start')).toBeVisible();
    await expect(page.locator('[data-kind="heard"]')).toHaveCount(0);
    await expect(other.locator('[data-kind="heard"]')).toHaveCount(0);
    await expect(page.getByTestId('audio-solve')).toBeDisabled();
    await page.getByTestId('audio-room').selectOption(second);
    await expect(page.locator('[data-kind="heard"]')).toContainText('multiplication');
    await page.reload();
    await expect(page.getByTestId('audio-room')).toHaveValue(second);
    await expect(page.locator('[data-kind="heard"]')).toContainText('multiplication');
  } finally {
    await other.close();
  }
});

test('audio capture fits a phone and sits directly below Capture in the main menu', async ({page}, info) => {
  await page.setViewportSize({width: 390, height: 844});
  await page.goto('/');
  await page.getByRole('button', {name: 'Open navigation'}).click();
  const navigation = page.getByRole('navigation', {name: 'Main navigation', exact: true});
  await expect(navigation.locator('[data-nav="home"] + [data-nav="audio"]')).toBeVisible();
  await page.getByTestId('nav-search').fill('microphone');
  await page.getByTestId('nav-results').getByRole('button', {name: /Audio Capture/}).click();
  await expect(page).toHaveURL(/\/audio$/);
  await expect(page.getByTestId('audio-start')).toBeVisible();
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  await page.screenshot({path: info.outputPath('audio-phone.png'), fullPage: true});
});

test('audio controls and guidance are translated in all six languages', async ({page}) => {
  await page.goto('/audio');
  const languages = ['ko', 'yue-HK', 'zh-CN', 'ja', 'es', 'cs'];
  for (const [index, language] of languages.entries()) {
    await page.getByTestId('language-select').selectOption(language);
    await expect(page.locator('html')).toHaveAttribute('lang', language);
    for (const [id, key] of [
      ['audio-start', 'Capture Audio'], ['audio-note', 'Note'],
      ['audio-solve', 'Solve'], ['audio-new-chat', 'New chat'],
      ['audio-stage', 'Not listening'],
    ]) {
      expect(translations[key][index]).not.toBe(key);
      await expect(page.getByTestId(id)).toHaveText(translations[key][index]);
    }
    for (const key of [
      'Open in Ask anything',
      'Speech is turned into text on your backend Mac. The cloud AI reads it only when you press Solve or Note.',
      "Solve answers the newest speech, with the earlier speech and answers in this chat as context. Note writes the whole session up as a note. Older speech is condensed, so the AI's context is never exceeded.",
    ]) {
      expect(translations[key][index]).not.toBe(key);
      await expect(page.getByText(translations[key][index], {exact: true})).toBeVisible();
    }
  }
});

test('speech drafts and microphone animation stream before the final transcript', async ({page}, info) => {
  await page.emulateMedia({reducedMotion: 'no-preference'});
  // Status must keep updating from SSE even when ordinary GET polling fails.
  await page.route('**/api/audio', async route => {
    if (route.request().method() === 'GET') await route.abort();
    else await route.continue();
  });
  const stream = page.waitForResponse(response => response.url().endsWith('/api/audio/events'));
  await page.goto('/audio');
  expect((await stream).headers()['content-type']).toContain('text/event-stream');
  await page.getByTestId('audio-start').click();
  await expect(page.getByTestId('audio-listening')).toBeVisible();
  await expect(page.getByTestId('audio-partial')).toContainText('Demo audio transcript');
  await expect(page.locator('[data-kind="heard"]')).toHaveCount(0);
  const orbit = page.locator('.audio-mic-orbit');
  expect(await orbit.evaluate(el => getComputedStyle(el, '::before').animationName)).toBe('audio-ripple');
  await expect.poll(() => page.getByTestId('audio-listening').evaluate(el => Number(el.style.getPropertyValue('--audio-level')))).toBeGreaterThan(0);
  await page.screenshot({path: info.outputPath('audio-streaming.png'), fullPage: true});
  await expect(page.locator('[data-kind="heard"]').first()).toContainText('simple arithmetic');
  await page.emulateMedia({reducedMotion: 'reduce'});
  expect(await orbit.evaluate(el => getComputedStyle(el, '::before').animationName)).toBe('none');
  await page.getByTestId('audio-stop').click();
  await expect(page.getByTestId('audio-listening')).toHaveCount(0);
  await expect(page.getByTestId('audio-partial')).toHaveCount(0);
});

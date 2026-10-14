import {test, expect} from '@playwright/test';
import fs from 'node:fs';

const storageKey = 'causewaybay.language.v1';
const languages = [
  ['en', 'English', 'New thought', 'Capture'],
  ['ko', '한국어', '새 생각', '캡처'],
  ['yue-HK', '廣東話', '新諗法', '擷取'],
  ['zh-CN', '简体中文', '新想法', '截屏'],
  ['ja', '日本語', '新しいアイデア', 'キャプチャ'],
  ['es', 'Español', 'Nueva idea', 'Capturar'],
  ['cs', 'Čeština', 'Nová myšlenka', 'Zachytit'],
];
const catalog = JSON.parse(fs.readFileSync('crates/frontend/src/locales.json', 'utf8'));
const translated = (key, code) => code === 'en' ? key : catalog[key][languages.findIndex(([id]) => id === code) - 1];

// The libraries and tools sit behind one fold ("More tools"); open it when needed.
async function nav(page, route) {
  const item = page.locator(`[data-nav="${route}"]`);
  if (!(await item.isVisible())) await page.locator('.more-tools > summary').click();
  await item.click();
}

test('translation catalog covers every marked label and preserves named placeholders', () => {
  for (const [key, values] of Object.entries(catalog)) {
    expect(values, key).toHaveLength(6);
    const placeholders = text => [...text.matchAll(/\{[^{}]+\}/g)].map(m => m[0]).sort();
    for (const value of values) {
      expect(value.trim(), key).not.toBe('');
      expect(placeholders(value), key).toEqual(placeholders(key));
    }
  }
  for (const file of ['main', 'chat', 'cloud', 'assistance', 'wiki', 'invisible', 'login', 'account', 'wallet', 'admin', 'dashboard', 'images', 'videos', 'i18n', 'theme']) {
    const source = fs.readFileSync(`crates/frontend/src/${file}.rs`, 'utf8');
    for (const match of source.split('#[cfg(test)]')[0].matchAll(/\btf?\(\s*("(?:[^"\\]|\\.)*")/g)) {
      const key = JSON.parse(match[1]);
      expect(Object.hasOwn(catalog, key), `${file}: ${key}`).toBe(true);
    }
  }
});

for (const [code, name, thought, capture] of languages) {
  test(`${name}: menus, chat, setup and dialogs translate and survive reload`, async ({page}) => {
    const errors = []; page.on('pageerror', error => errors.push(error.message));
    await page.goto('/');
    const selector = page.getByTestId('language-select');
    await expect(selector.locator('option')).toHaveCount(7);
    await selector.selectOption(code);
    await expect(page.locator('html')).toHaveAttribute('lang', code);
    await expect(page).toHaveTitle(translated('Invisible AI · Your thinking space', code));
    await expect(page.getByRole('heading', {name:thought, exact:true})).toBeVisible();
    await expect(page.getByRole('button', {name:capture, exact:true})).toBeVisible();
    for (const key of ['Multi Capture', 'Delete all', 'Copy all text']) {
      await expect(page.getByRole('button', {name:translated(key, code), exact:true})).toBeVisible();
    }
    await expect(page.locator('.chat-dock textarea')).toHaveAttribute('aria-label', translated('Chat message', code));
    expect(await page.evaluate(key => localStorage.getItem(key), storageKey)).toBe(code);
    await page.reload();
    await expect(selector).toHaveValue(code);
    await expect(page.getByRole('heading', {name:thought, exact:true})).toBeVisible();
    await page.getByRole('button', {name:translated('Clear chatroom', code), exact:true}).click();
    await expect(page.getByText(translated('Permanently clear all messages, captures, files, and results in this chatroom?', code), {exact:true})).toBeVisible();
    await page.getByRole('button', {name:translated('Cancel', code), exact:true}).click();
    await nav(page, 'library');
    await page.getByRole('button', {name:translated('New memo', code), exact:true}).click();
    await expect(page.getByRole('dialog')).toContainText(translated('Give it a title', code));
    await expect(page.getByLabel(translated('Memo content', code), {exact:true})).toBeVisible();
    await page.getByRole('button', {name:translated('Close dialog', code), exact:true}).click();
    for (const [route, heading] of [['files','Everything, in its place.'], ['images','Every captured moment.'], ['devices','Your connected devices.']]) {
      await nav(page, route);
      await expect(page.getByRole('heading', {name:translated(heading,code), exact:true})).toBeVisible();
    }
    await expect(page.getByLabel(translated('Capture client command',code))).toHaveText('./bin/client http://127.0.0.1:3099');
    await page.getByRole('button', {name:translated('Open settings',code), exact:true}).click();
    await expect(page.getByRole('dialog')).toContainText(translated('API keys stay on the backend', code));
    expect(errors).toEqual([]);
  });
}

test('switching language preserves draft, room, attachment, original messages and the live stream', async ({page}) => {
  await page.addInitScript(() => {
    const Native = window.EventSource;
    window.roomSubscriptions = 0;
    window.EventSource = class extends Native {
      constructor(url, options) { super(url, options); if(String(url).includes('/api/rooms/')) window.roomSubscriptions++; }
    };
  });
  await page.goto('/chat');
  await page.getByRole('button',{name:'New chat',exact:true}).click();
  const room = page.locator('.chat-toolbar select');
  await expect(room).not.toHaveValue('');
  await page.getByRole('button', {name:'Rename chatroom'}).click();
  await page.getByLabel('Chatroom name').fill('Files');
  await page.getByRole('button', {name:'Save name'}).click();
  await expect(room.locator('option:checked')).toHaveText('Files');
  const id = await room.inputValue();
  const composer = page.locator('.chat-dock textarea');
  await composer.fill('Delete'); await composer.press('Enter');
  await expect(page.locator('.from-user .chat-user-text')).toHaveText('Delete');
  await expect(page.locator('.from-ai .answer-content')).not.toBeEmpty();
  await expect(page.getByRole('button',{name:'Stop answer',exact:true})).toHaveCount(0);
  const answer = await page.locator('.from-ai .answer-content').textContent();
  await page.getByLabel('Attach to chat').setInputFiles({name:'English 한국어.txt', mimeType:'text/plain', buffer:Buffer.from('Original text')});
  await expect(page.locator('.chat-attachment')).toContainText('English 한국어.txt');
  const subscriptions = await page.evaluate(() => window.roomSubscriptions);
  for (const [code] of languages) {
    await composer.fill('Unsent 日本語 draft');
    await page.getByTestId('language-select').selectOption(code);
    await expect(composer).toHaveValue('Unsent 日本語 draft');
    await expect(room).toHaveValue(id);
    await expect(room.locator('option:checked')).toHaveText('Files');
    await expect(page.locator('.chat-attachment')).toContainText('English 한국어.txt');
    await expect(page.locator('.from-user .chat-user-text')).toHaveText('Delete');
    await expect(page.locator('.from-ai .answer-content')).toHaveText(answer);
  }
  expect(await page.evaluate(() => window.roomSubscriptions)).toBe(subscriptions);
  await page.getByTestId('language-select').selectOption('ko');
  await page.locator('.from-user').getByRole('button',{name:'메시지 수정'}).click();
  await page.getByRole('textbox',{name:'메시지 수정'}).fill('수정한 메시지');
  await page.getByRole('button',{name:'메시지 저장'}).click();
  await expect(page.locator('.from-user .chat-user-text')).toHaveText('수정한 메시지');
});

test('language syncs across tabs and resets safely if the stored choice is removed or invalid', async ({page, context}) => {
  await context.addInitScript(() => {
    const Native = window.EventSource;
    window.notificationConnections = 0;
    window.EventSource = class extends Native {
      constructor(url, options) { super(url, options); if (String(url).includes('/api/notifications')) window.notificationConnections++; }
    };
  });
  const second = await context.newPage();
  await page.goto('/'); await second.goto('/');
  await expect(page.getByTestId('language-select')).toHaveValue('en');
  await second.locator('.chat-dock textarea').fill('Keep this draft');
  await page.getByTestId('language-select').selectOption('ja');
  await expect(second.getByTestId('language-select')).toHaveValue('ja');
  await expect(second.getByRole('heading',{name:'新しいアイデア',exact:true})).toBeVisible();
  await expect(second.locator('.chat-dock textarea')).toHaveValue('Keep this draft');
  expect(await page.evaluate(()=>window.notificationConnections)).toBe(1);
  expect(await second.evaluate(()=>window.notificationConnections)).toBe(1);
  await page.evaluate(key=>localStorage.removeItem(key),storageKey);
  await expect(second.getByTestId('language-select')).toHaveValue('en');
  await page.evaluate(key=>localStorage.setItem(key,'invalid-language'),storageKey);
  await page.reload();
  await expect(page.getByTestId('language-select')).toHaveValue('en');
  await expect(page.locator('html')).toHaveAttribute('lang','en');
  await second.close();
});

test('blocked browser storage keeps language usable for the current session', async ({page}) => {
  await page.addInitScript(() => Object.defineProperty(window,'localStorage',{get(){throw new DOMException('Blocked','SecurityError');}}));
  await page.goto('/');
  await page.getByTestId('language-select').selectOption('es');
  await expect(page.getByRole('heading',{name:'Nueva idea',exact:true})).toBeVisible();
  await expect(page.getByRole('status').filter({hasText:'El almacenamiento del navegador no está disponible'})).toContainText('El almacenamiento del navegador no está disponible');
  await nav(page, 'files');
  await expect(page.getByRole('heading',{name:'Cada cosa en su sitio.',exact:true})).toBeVisible();
});

test('all seven languages fit the mobile menu and chat controls', async ({page}) => {
  await page.setViewportSize({width:390,height:844});
  await page.goto('/');
  for (const [code, , ,capture] of languages) {
    await page.getByTestId('language-select').selectOption(code);
    await page.getByRole('button',{name:translated('Open navigation',code)}).click();
    await expect(page.locator('[data-nav="devices"]')).toBeVisible();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.keyboard.press('Escape');
    await expect(page.getByRole('button',{name:capture,exact:true})).toBeVisible();
    expect(await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth)).toBe(true);
    await page.screenshot({path:`test-results/language-${code}-mobile.png`,fullPage:true});
  }
});

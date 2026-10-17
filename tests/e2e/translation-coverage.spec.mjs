import {test, expect} from '@playwright/test';
import fs from 'node:fs';
import path from 'node:path';
import {openConversationOptions, openNav} from './workspace-controls.mjs';

// Every non-English language, every page and the main dialogs: any rendered text
// that is still English UI copy is reported. Two sources feed the check:
//   1. the translation catalog (a key whose value for the language is the key
//      itself was never translated),
//   2. every string literal in the frontend sources (text rendered without t(),
//      or passed to t() with a key the catalog does not have).
// CJK languages get a third, heuristic net: a run of Latin words that no source
// literal explains (server-provided or JavaScript-provided copy).
const storageKey = 'causewaybay.language.v1';
const languages = [
  ['ko', '한국어', 0, true],
  ['yue-HK', '廣東話', 1, true],
  ['zh-CN', '简体中文', 2, true],
  ['ja', '日本語', 3, true],
  ['es', 'Español', 4, false],
  ['cs', 'Čeština', 5, false],
];
const routes = ['/', '/chat', '/save', '/notes', '/wiki', '/autonomous', '/assistance', '/overview', '/library', '/files', '/images', '/videos', '/code', '/coding', '/skills', '/dashboard', '/devices', '/admin', '/quiz', '/ai', '/wallet'];
const sourceDir = 'crates/frontend/src';
const catalog = JSON.parse(fs.readFileSync(path.join(sourceDir, 'locales.json'), 'utf8'));
const sources = fs.readdirSync(sourceDir).filter(f => f.endsWith('.rs')).map(f => [f, fs.readFileSync(path.join(sourceDir, f), 'utf8').split('#[cfg(test)]')[0]]);
const translated = (key, index) => Object.hasOwn(catalog, key) ? catalog[key][index] : key;

// Product and protocol names that stay as they are in every language.
const keptAsIs = new Set(['TextEdit', 'LLM', 'Codex', 'Claude Code', 'Codex / Claude Code', 'AI', 'Tailscale DNS', 'Tailscale IP', 'Testnet', 'Mainnet', 'Mint', 'Faucet', 'Q&A', 'Wiki', 'Causewaybay', 'Invisible AI', 'INVISIBLE AI', 'MLX', 'Apple', 'Anthropic', 'OpenAI', 'Gemini', 'Grok', 'Ollama', 'Terminal', 'Notes', 'Mac', 'OCR', 'GPU', 'API', 'URL', 'Auto', 'Chat', 'Cloud', 'Video', 'Error', 'tokens', 'est.', 'test', 'Hybrid', 'Personal · Invisible AI', 'Terminal (ai- …)', 'Original', 'Agent', 'Model', 'Offline', 'online', 'API URL', 'Local AI', 'Cloud AI', 'USDC', 'CRONOS MAINNET', 'CRONOS TESTNET', 'Cronos', 'Windows', 'macOS', 'Linux', 'iOS', 'iPadOS', 'Android', '⌘ Enter', 'Whisper', 'MLX Whisper', 'Apple Speech', 'Cronos test coin']);
// English defaults the server stores as data (room titles) and the UI shows as they are.
const serverDefaults = new Set(['New thought']);

// Every string literal in the frontend sources that could be rendered text.
function sourceLiterals() {
  const literals = new Map();
  for (const [file, source] of sources) {
    for (const match of source.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)) {
      const text = match[1].replace(/\\"/g, '"').replace(/\\n/g, '\n').replace(/\\u\{([0-9a-f]+)\}/gi, (_, hex) => String.fromCodePoint(parseInt(hex, 16)));
      if (!/[A-Za-z]{2}/.test(text)) continue;
      if (/^[a-z0-9_.:/\-]+$/.test(text)) continue; // ids, classes, routes, urls
      if (!literals.has(text)) literals.set(text, file);
    }
  }
  return literals;
}
// Keys handed to t()/tf(), including both branches of a conditional argument.
const spans = new Map(); // file → [start, end] of every t()/tf() argument
function translationKeys() {
  const keys = new Map();
  const string = /"(?:[^"\\]|\\.)*"/y;
  for (const [file, source] of sources) {
    spans.set(file, []);
    for (const call of source.matchAll(/\btf?\(/g)) {
      let depth = 1, i = call.index + call[0].length;
      const start = i;
      while (i < source.length && depth) {
        if (source[i] === '"') { string.lastIndex = i; string.exec(source); i = string.lastIndex; continue; }
        if (source[i] === '(') depth++; else if (source[i] === ')') depth--;
        i++;
      }
      spans.get(file).push([start, i]);
      const argument = source.slice(start, i - 1).split(',&[')[0];
      for (const literal of argument.match(/"(?:[^"\\]|\\.)*"/g) ?? []) {
        const key = JSON.parse(literal);
        if (/[A-Za-z]{2}/.test(key) && !/^[a-z0-9_.:/\-]+$/.test(key) && !keys.has(key)) keys.set(key, file);
      }
    }
  }
  return keys;
}
const literals = sourceLiterals();
const keys = translationKeys();
// English the server or the shared crate composes and the frontend shows as it
// is (status lines, model notes). The demo seed's memos are user content, not UI.
function backendLiterals() {
  const ui = [], seed = [];
  for (const dir of ['crates/server/src', 'crates/shared/src']) {
    for (const file of fs.readdirSync(dir).filter(f => f.endsWith('.rs'))) {
      const source = fs.readFileSync(path.join(dir, file), 'utf8');
      const seedStart = dir === 'crates/server/src' && file === 'lib.rs' ? source.indexOf('\nfn seed(') : -1;
      const seedEnd = seedStart >= 0 ? (source.indexOf('\n}\n', seedStart) + 1 || source.length) : -1;
      for (const match of source.matchAll(/"((?:[^"\\\n]|\\.)*)"/g)) {
        const text = match[1].replace(/\\"/g, '"');
        if (latinWords(text) < 2) continue;
        if (seedStart >= 0 && match.index > seedStart && match.index < seedEnd) seed.push(text);
        else ui.push(text);
      }
    }
  }
  return {ui, seed};
}
const latinWords = (text) => (text.match(/[A-Za-z]{2,}/g) ?? []).length;
const backend = backendLiterals();

const reportDir = process.env.PW_OUTPUT || 'test-results';
const report = {run: process.env.TEST_WORKER_INDEX === undefined ? 'local' : process.env.PW_RUN ?? String(process.ppid), generated: new Date().toISOString(), static: {}, languages: {}};
const reportFile = path.join(reportDir, 'translation-coverage.json');
// Playwright starts a fresh worker after a failed test, so merge with what
// earlier tests of this run already wrote.
function saveReport() {
  fs.mkdirSync(reportDir, {recursive: true});
  let previous = {};
  try { previous = JSON.parse(fs.readFileSync(reportFile, 'utf8')); } catch {}
  if (previous.run !== report.run) previous = {};
  const merged = {...previous, ...report, static: Object.keys(report.static).length ? report.static : previous.static ?? {}, languages: {...previous.languages, ...report.languages}};
  fs.writeFileSync(reportFile, JSON.stringify(merged, null, 2));
}

test('every t() key exists in the catalog and every catalog entry is translated into every language', () => {
  const missing = [...keys].filter(([key]) => !Object.hasOwn(catalog, key) && !keptAsIs.has(key)).map(([key, file]) => `${file}: ${key}`);
  const untranslated = {};
  for (const [key, values] of Object.entries(catalog)) {
    if (keptAsIs.has(key)) continue;
    const same = languages.filter(([, , index]) => values[index] === key).map(([code]) => code);
    if (same.length) untranslated[key] = same;
  }
  const rawInHtml = [];
  for (const [file, source] of sources) {
    for (const match of source.matchAll(/\{"([A-Z][^"\\]{2,})"\}/g)) {
      if (spans.get(file).some(([start, end]) => match.index >= start && match.index < end)) continue; // a branch of t(if …)
      rawInHtml.push(`${file}:${source.slice(0, match.index).split('\n').length}: ${match[1]}`);
    }
  }
  report.static = {missingKeys: missing, untranslatedCatalogEntries: untranslated, rawLiteralsInHtml: rawInHtml};
  saveReport();
  expect.soft(missing, 'translation keys missing from locales.json').toEqual([]);
  expect.soft(Object.keys(untranslated), 'catalog entries still English in some language').toEqual([]);
  expect.soft(rawInHtml, 'text rendered without t()').toEqual([]);
});

// Text a reader sees or a screen reader announces on the page right now.
async function visibleText(page) {
  return page.evaluate(() => {
    const skip = (element) => element.closest('script, style, code, pre, kbd, .answer-content, .chat-user-text, .memo-body, [data-testid="language-select"], .language-picker');
    const texts = new Set();
    const add = (text) => { const t = (text ?? '').replace(/\s+/g, ' ').trim(); if (t) texts.add(t); };
    for (const details of document.querySelectorAll('details')) details.open = true;
    const walker = document.createTreeWalker(document.body, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (!skip(node.parentElement)) add(node.nodeValue);
    }
    for (const element of document.body.querySelectorAll('[aria-label], [title], [placeholder], [alt]')) {
      if (skip(element)) continue;
      for (const name of ['aria-label', 'title', 'placeholder', 'alt']) add(element.getAttribute(name));
    }
    add(document.title);
    return [...texts];
  });
}
const hasCjk = (text) => /[\u1100-\u11ff\u3040-\u30ff\u3130-\u318f\u3400-\u4dbf\u4e00-\u9fff\uac00-\ud7af]/.test(text);

function classify(text, index, cjk) {
  if (keptAsIs.has(text)) return null;
  if (serverDefaults.has(text)) return 'server-default';
  if (Object.hasOwn(catalog, text)) {
    return catalog[text][index] === text ? 'english-in-catalog' : 'rendered-without-t';
  }
  if (keys.has(text)) return 'key-missing-from-catalog';
  if (literals.has(text)) return 'rendered-without-t';
  if (!cjk || hasCjk(text) || latinWords(text) < 3 || text.startsWith('/') || !/[A-Z]/.test(text)) return null;
  if (backend.seed.some(literal => text.startsWith(literal.slice(0, 30)))) return null; // demo memos
  // Composed text: "uses 5.2 GB · fits GPU memory" is several t() pieces, and
  // "Rust runs with …" is a tf() template whose placeholders were filled in.
  const piece = (candidate) => latinWords(candidate) >= 2 && (candidate.includes('{')
    ? new RegExp('^' + candidate.split(/\{[^{}]+\}/).map(part => part.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')).join('.+') + '$').test(text)
    : text.includes(candidate));
  if ([...keys.keys()].some(key => !Object.hasOwn(catalog, key) && piece(key))) return 'key-missing-from-catalog';
  if (Object.keys(catalog).some(key => catalog[key][index] === key && piece(key))) return 'english-in-catalog';
  if (backend.ui.some(literal => latinWords(literal) >= 3 ? text.includes(literal) : text.startsWith(literal) || text.endsWith(literal))) return 'server-provided';
  return 'unknown-source';
}

for (const [code, name, index, cjk] of languages) {
  test(`${name}: no English UI text on any page or dialog`, async ({page}) => {
    test.setTimeout(240000);
    await page.addInitScript(([key, value]) => localStorage.setItem(key, value), [storageKey, code]);
    const hits = [];
    const seen = new Map();
    const scan = async (where) => {
      for (const text of await visibleText(page)) {
        const kind = classify(text, index, cjk);
        if (!kind) continue;
        if (!seen.has(text)) seen.set(text, {text, kind, source: keys.get(text) ?? literals.get(text) ?? null, where: []});
        if (!seen.get(text).where.includes(where)) seen.get(text).where.push(where);
      }
    };
    for (const route of routes) {
      await page.goto(route);
      await expect(page.locator('html')).toHaveAttribute('lang', code);
      await page.locator('main, .login-card, .app-shell').first().waitFor({state: 'attached'});
      await page.waitForTimeout(1500);
      await scan(route);
      if (route === '/') {
        await openConversationOptions(page);
        const clear = page.getByRole('button', {name: translated('Clear chatroom', index), exact: true});
        if (await clear.isVisible().catch(() => false)) {
          await clear.click();
          await page.waitForTimeout(300);
          await scan('/ (clear chatroom dialog)');
          await page.getByRole('button', {name: translated('Cancel', index), exact: true}).click();
        }
      }
      if (route === '/library') {
        const memo = page.getByRole('button', {name: translated('New memo', index), exact: true});
        if (await memo.isVisible().catch(() => false)) {
          await memo.click();
          await page.waitForTimeout(300);
          await scan('/library (new memo dialog)');
          await page.getByRole('button', {name: translated('Close dialog', index), exact: true}).click();
        }
      }
      if (route === '/devices') {
        const settings = page.getByRole('button', {name: translated('Open settings', index), exact: true});
        if (await settings.isVisible().catch(() => false)) {
          await settings.click();
          await page.waitForTimeout(800);
          await scan('/devices (backend settings dialog)');
        }
      }
    }
    hits.push(...seen.values());
    hits.sort((a, b) => a.kind.localeCompare(b.kind) || a.text.localeCompare(b.text));
    const byKind = {};
    for (const hit of hits) byKind[hit.kind] = (byKind[hit.kind] ?? 0) + 1;
    report.languages[code] = {name, total: hits.length, byKind, hits};
    saveReport();
    expect.soft(hits.map(h => `[${h.kind}] ${h.text} ← ${h.where.join(', ')}`), `${name}: English UI text`).toEqual([]);
  });
}

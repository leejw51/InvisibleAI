import {test, expect} from '@playwright/test';
import {isolatedBackend} from './invisible-fixture.mjs';
isolatedBackend(test, 3177);
test.beforeEach(async ({request}) => {
  expect((await request.delete('/api/user/data')).ok()).toBeTruthy();
});
test.setTimeout(120000);

// The coding agent: a project is a folder on the backend, the agent writes
// into it, the backend builds and runs it, the output comes back to the page,
// and a finished program can be published as a skill the chat can run.
test('Coding creates a sandboxed project, the agent writes and runs it, the user edits, runs with input, and publishes a skill', async ({page, request}, info) => {
  const overview = await (await request.get('/api/coding')).json();
  expect(overview.languages.map(l => l.id)).toEqual(['rust', 'go', 'python', 'c', 'lua', 'typescript']);
  expect(overview.ai.engine).toBe('harness');
  const python = overview.languages.find(l => l.id === 'python');
  test.skip(!python.available, `Python is not available on this backend: ${python.detail}`);

  // The menu entry, behind the fold; the page knows which AI does the work.
  await page.goto('/');
  const toggle = page.getByRole('button', {name: 'Open navigation'});
  if (await toggle.isVisible()) await toggle.click();
  if (!(await page.locator('[data-nav="coding"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.locator('[data-nav="coding"]').click();
  await expect(page).toHaveURL(/\/coding$/);
  await expect(page.locator('.breadcrumbs strong')).toHaveText('Coding');
  await expect(page.getByTestId('coding-ai')).toContainText('Demo responses');
  await expect(page.getByRole('group', {name: 'Language'}).getByRole('button')).toHaveCount(6);
  // The AI is picked right here, with the same picker every AI page shares; the
  // pick lands in this browser and in the account, and the AI settings page agrees.
  const picker = page.getByTestId('coding-ai-picker');
  await picker.getByLabel('Coding AI provider').selectOption('openai');
  await picker.getByLabel('Coding AI model').fill('coding-test-model');
  await expect(page.getByTestId('coding-ai')).toContainText('Cloud AI · openai · coding-test-model');
  await expect.poll(async () => (await (await request.get('/api/ai/preferences')).json()).cloud_provider).toBe('openai');
  expect(JSON.parse(await page.evaluate(() => localStorage.getItem('causewaybay.chat.local'))).models.openai).toBe('coding-test-model');
  await page.goto('/ai');
  await expect(page.locator('.breadcrumbs strong')).toHaveText('AI settings');
  await expect(page.getByTestId('settings-ai-picker').getByLabel('Settings AI provider')).toHaveValue('openai');
  await expect(page.getByTestId('settings-ai-picker').getByLabel('Settings AI model')).toHaveValue('coding-test-model');
  await page.getByTestId('settings-ai-picker').getByRole('group', {name: 'AI mode'}).getByRole('button', {name: 'Hybrid'}).click();
  await expect.poll(async () => (await (await request.get('/api/ai/preferences')).json()).mode).toBe('hybrid');
  await page.getByTestId('settings-ai-picker').getByLabel('Settings AI provider').selectOption('openai');
  await page.goto('/coding');

  // A new Python project: a real folder with a scaffold, opened at once.
  await page.getByRole('group', {name: 'Language'}).getByRole('button', {name: /^Python/}).click();
  await page.getByLabel('Project name').fill('Word counter');
  await page.getByTestId('coding-create').click();
  await expect(page).toHaveURL(/\/coding\/[0-9a-f-]{36}$/);
  const id = page.url().split('/').pop();
  await expect(page.getByTestId('coding-title')).toHaveText('Word counter');
  await expect(page.getByTestId('coding-files')).toContainText('main.py');
  await expect(page.getByTestId('coding-source')).toHaveValue(/Hello, world!/);
  const detail = await (await request.get(`/api/coding/projects/${id}`)).json();
  expect(detail.folder.endsWith(`/projects/${id}`)).toBeTruthy();
  expect(detail.files.map(f => f.path)).toEqual(['.gitignore', 'README.md', 'main.py']);

  // Asking the agent: the work shows live (file written, run), then the answer.
  await page.getByTestId('coding-prompt').fill('Print hello');
  await page.getByTestId('coding-ask').click();
  await expect(page.getByTestId('coding-live')).toBeVisible();
  await expect(page.getByTestId('coding-answer')).toContainText('Run result', {timeout: 60000});
  await expect(page.getByTestId('coding-answer')).toContainText('Hello, world!');
  await expect(page.getByTestId('coding-live').locator('.tool-event')).toContainText(['Writing main.py', 'Run succeeded']);
  await expect(page.getByTestId('coding-ask')).toBeEnabled();
  const memos = (await (await request.get(`/api/coding/projects/${id}`)).json()).memos;
  expect(memos).toHaveLength(1);
  expect(memos[0].kind).toBe('coding');
  expect(memos[0].status).toBe('complete');
  expect(memos[0].provider).toBe('openai');
  expect(memos[0].model).toBe('coding-test-model');
  await page.screenshot({path: `test-results/coding-${info.project.name}.png`, fullPage: true});

  // The user edits the program, saves it, and runs it with input.
  await page.getByTestId('coding-files').getByRole('button', {name: /main\.py/}).click();
  await page.getByTestId('coding-source').fill('import sys\nwords = sys.stdin.read().split()\nprint(len(words))\n');
  await page.getByTestId('coding-save').click();
  await expect(page.getByTestId('coding-save')).toBeDisabled();
  await page.getByTestId('coding-stdin').fill('one two three four');
  await page.getByTestId('coding-run').click();
  await expect(page.getByTestId('coding-output')).toContainText('Succeeded');
  await expect(page.getByTestId('coding-stdout')).toHaveText(/^4\s*$/);
  // Every save and agent request is a checkpoint; reverting brings the earlier file back, as a new checkpoint.
  // (The demo agent rewrites the scaffold unchanged, so its request adds no checkpoint.)
  const history = (await (await request.get(`/api/coding/projects/${id}/history`)).json()).history;
  expect(history.length).toBeGreaterThanOrEqual(2);
  expect(history[0].message).toBe('Edited main.py');
  const first = history.find(h => h.message === 'New project');
  expect(first).toBeTruthy();
  const reverted = await (await request.post(`/api/coding/projects/${id}/revert`, {data: {commit: first.commit}})).json();
  expect(reverted.history[0].message).toMatch(/^Reverted to /);
  expect((await (await request.get(`/api/coding/projects/${id}/file?path=main.py`)).json()).text).toContain('Hello, world!');
  expect((await request.post(`/api/coding/projects/${id}/revert`, {data: {commit: 'deadbeef'}})).status()).toBe(400);
  // Back to the counting program for the rest of the flow, through the page.
  await page.reload();
  await page.getByTestId('coding-files').getByRole('button', {name: /main\.py/}).click();
  await expect(page.getByTestId('coding-source')).toHaveValue(/Hello, world!/);
  await page.getByTestId('coding-source').fill('import sys\nwords = sys.stdin.read().split()\nprint(len(words))\n');
  await page.getByTestId('coding-save').click();
  await expect(page.getByTestId('coding-save')).toBeDisabled();
  await expect(page.getByTestId('coding-checkpoints')).toContainText('Edited main.py');
  const file = await (await request.get(`/api/coding/projects/${id}/file?path=main.py`)).json();
  expect(file.text).toContain('sys.stdin.read()');
  // Tests run in the same sandbox from the Run tests button.
  expect((await request.post(`/api/coding/projects/${id}/file`, {data: {path: 'test_main.py', text: 'import unittest\n\nclass T(unittest.TestCase):\n    def test_truth(self):\n        self.assertEqual(1 + 1, 2)\n'}})).ok()).toBeTruthy();
  await page.getByTestId('coding-test').click();
  await expect(page.getByTestId('coding-tests')).toContainText('Tests passed', {timeout: 60000});
  const tested = await (await request.post(`/api/coding/projects/${id}/test`)).json();
  expect(tested.success).toBe(true);
  expect(tested.phase).toBe('test');
  // Suggest input asks the AI for an example run (a fixture in demo) and fills the Run box.
  await page.getByTestId('coding-suggest').click();
  await expect(page.getByTestId('coding-stdin')).toHaveValue(/example for Word counter/);
  const withExample = await (await request.get(`/api/coding/projects/${id}`)).json();
  expect(withExample.example.stdin).toContain('example for Word counter');
  expect(withExample.files.map(f => f.path)).toContain('example.json');
  // Paths cannot leave the folder, and the backend's own folders are off limits.
  expect((await request.get(`/api/coding/projects/${id}/file?path=../../causewaybay.sqlite3`)).status()).toBe(400);
  expect((await request.post(`/api/coding/projects/${id}/file`, {data: {path: '.cache/x', text: 'no'}})).status()).toBe(400);

  // Published as a skill: listed on Skills, runnable there with input.
  await page.getByTestId('coding-skill-name').fill('word count');
  await page.getByTestId('coding-skill-description').fill('Counts the words in the text it is given');
  await page.getByTestId('coding-publish').click();
  await expect(page.getByTestId('coding-skill')).toContainText('Published as word-count');
  await expect(page.getByTestId('coding-retire')).toBeVisible();
  const skills = await (await request.get('/api/skills')).json();
  expect(skills.skills.map(s => s.name)).toEqual(['word-count']);
  if (await toggle.isVisible()) await toggle.click();
  if (!(await page.locator('[data-nav="skills"]').isVisible())) await page.locator('.more-tools > summary').click();
  await page.locator('[data-nav="skills"]').click();
  await expect(page).toHaveURL(/\/skills$/);
  const card = page.getByTestId('skill-card').filter({hasText: 'word-count'});
  await expect(card).toHaveCount(1);
  await expect(card).toContainText('Counts the words');
  await expect(card.getByTestId('skill-state')).toHaveText(/Live/);
  await card.getByTestId('skill-input').fill('a b c');
  await card.getByTestId('skill-run').click();
  await expect(card.getByTestId('skill-output')).toContainText('Succeeded');
  await expect(card.getByTestId('skill-output').locator('.coding-stdout')).toHaveText(/^3\s*$/);
  const run = await (await request.post(`/api/skills/${id}/run`, {data: {input: 'x y'}})).json();
  expect(run.success).toBe(true);
  expect(run.stdout.trim()).toBe('2');
  expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBeTruthy();
  // Managing the skill in place: rename and re-describe, retire, add back from the project list.
  // The run is in the ledger; the live state is unchanged by it.
  await expect(card.getByTestId('skill-stats')).toContainText('1 runs · 0 failed');
  await card.getByTestId('skill-edit').click();
  // While editing, the card shows the form (its text is in inputs), so find the form itself.
  const editForm = page.getByTestId('skills-edit-form');
  await editForm.getByLabel('Skill name').fill('count words');
  await editForm.getByLabel('What it does and what input it reads').fill('Counts words on stdin');
  await editForm.getByTestId('skills-edit-save').click();
  const renamed = page.getByTestId('skill-card').filter({hasText: 'count-words'});
  await expect(renamed).toHaveCount(1);
  await expect(renamed).toContainText('Counts words on stdin');
  await renamed.getByTestId('skill-retire').click();
  await expect(page.getByTestId('skill-card')).toHaveCount(0);
  const draft = page.getByTestId('skills-projects').locator('li').filter({hasText: 'Word counter'});
  await expect(draft).toContainText('runs cleanly');
  await draft.getByTestId('skills-add').click();
  const addForm = page.getByTestId('skills-add-form');
  await addForm.getByLabel('What it does and what input it reads').fill('Counts words again');
  await addForm.getByTestId('skills-add-save').click();
  const again = page.getByTestId('skill-card').filter({hasText: 'word-counter'});
  await expect(again).toHaveCount(1);
  // The code opens in the workbench; deleting from the skill card removes code, skill and card.
  await again.getByTestId('skill-open').click();
  await expect(page).toHaveURL(`/coding/${id}`);
  await expect(page.getByTestId('coding-title')).toHaveText('Word counter');
  await page.goto('/skills');
  const toDelete = page.getByTestId('skill-card').filter({hasText: 'word-counter'});
  await toDelete.getByTestId('skill-delete').click();
  await toDelete.getByTestId('skill-delete').click();
  await expect(page.getByTestId('skill-card')).toHaveCount(0);
  expect((await (await request.get('/api/skills')).json()).skills).toEqual([]);
  expect((await request.get(`/api/coding/projects/${id}`)).status()).toBe(404);
});

test('A skill is made from a description, goes live when it runs, and the chat catalogue lists only live skills', async ({page, request}) => {
  const overview = await (await request.get('/api/coding')).json();
  test.skip(!overview.languages.find(l => l.id === 'python').available, 'Python is not available on this backend');
  await page.goto('/skills');
  await expect(page.locator('.breadcrumbs strong')).toHaveText('Skills');
  await page.getByTestId('skills-new').click();
  await page.getByTestId('skills-name').fill('Shout');
  await page.getByTestId('skills-description').fill('Upper-cases the text on stdin');
  await page.getByRole('group', {name: 'Language'}).getByRole('button', {name: /^Python/}).click();
  await page.getByTestId('skills-create').click();
  // The agent builds it in the workbench; the skill is listed at once and goes live after the run.
  await expect(page).toHaveURL(/\/coding\/[0-9a-f-]{36}$/);
  const id = page.url().split('/').pop();
  await expect(page.getByTestId('coding-title')).toHaveText('shout');
  await expect(page.getByTestId('coding-answer')).toContainText('Run result', {timeout: 60000});
  await expect.poll(async () => (await (await request.get('/api/skills')).json()).skills.find(s => s.id === id)?.ready).toBe(true);
  const skills = await (await request.get('/api/skills')).json();
  expect(skills.skills.map(s => [s.name, s.ready, s.building])).toEqual([['shout', true, false]]);
  // A skill without a clean run is listed but not offered to the chat.
  const draft = await (await request.post('/api/skills', {data: {name: 'broken', description: 'Never runs', language: 'lua'}})).json();
  expect(draft.project.skill.name).toBe('broken');
  await expect.poll(async () => (await (await request.get(`/api/coding/projects/${draft.project.id}`)).json()).memos[0].status).not.toBe('running');
  const names = (await (await request.get('/api/skills')).json()).skills.map(s => s.name).sort();
  expect(names).toEqual(['broken', 'shout']);
  expect((await request.post('/api/skills', {data: {name: 'shout', description: 'duplicate', language: 'python'}})).status()).toBe(400);
  expect((await request.post('/api/skills', {data: {name: 'x', description: 'short name', language: 'python'}})).status()).toBe(400);
  await page.goto('/skills');
  await expect(page.getByTestId('skill-card').filter({hasText: 'shout'}).getByTestId('skill-state')).toHaveText(/Live/);
  const run = await (await request.post(`/api/skills/${id}/run`, {data: {input: 'hi there', args: []}})).json();
  expect(run.success).toBe(true);
});

test('Projects and skills belong to the account and refuse unknown languages', async ({request}) => {
  expect((await request.post('/api/coding/projects', {data: {language: 'cobol'}})).status()).toBe(400);
  const created = await (await request.post('/api/coding/projects', {data: {language: 'lua', name: 'Lua one'}})).json();
  expect(created.language_name).toBe('Lua');
  expect(created.entry).toBe('main.lua');
  expect(created.skill).toBeNull();
  const list = (await (await request.get('/api/coding')).json()).projects;
  expect(list.map(p => p.name)).toEqual(['Lua one']);
  // A skill needs a name and a description; retiring clears it.
  expect((await request.patch(`/api/coding/projects/${created.id}`, {data: {skill: {name: 'x', description: 'too short name'}}})).status()).toBe(400);
  const published = await (await request.patch(`/api/coding/projects/${created.id}`, {data: {skill: {name: 'Lua One', description: 'Says hello'}}})).json();
  expect(published.skill).toEqual({name: 'lua-one', description: 'Says hello'});
  const retired = await (await request.patch(`/api/coding/projects/${created.id}`, {data: {skill: null}})).json();
  expect(retired.skill).toBeNull();
  expect((await request.delete(`/api/coding/projects/${created.id}`)).ok()).toBeTruthy();
});

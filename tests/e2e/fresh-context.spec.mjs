import {test, expect} from '@playwright/test';

const roomPicker = page => page.getByRole('combobox', {name:'Chat room', exact:true});
const prompt = page => page.getByRole('textbox', {name:'Chat message', exact:true});
const button = (page, name) => page.getByRole('button', {name, exact:true});
async function startNew(page, name) {
  if (name === 'New thought') {
    const navigation = button(page, 'Open navigation');
    if (await navigation.isVisible() && await navigation.getAttribute('aria-expanded') !== 'true') await navigation.click();
  }
  await button(page, name).click();
}


for (const action of ['New thought', 'New chat']) {
  test(`${action} clears the previous problem, draft and photos before sending another problem`, async ({page, request}) => {
    const submissions = [];
    page.on('websocket', ws => ws.on('framesent', frame => {
      const message = JSON.parse(String(frame.payload));
      if (message.path === '/api/think') submissions.push(message.body);
    }));
    await page.goto('/');
    await expect(roomPicker(page)).not.toHaveValue('');
    const previous = await roomPicker(page).inputValue();
    await prompt(page).fill('OLD_PROBLEM');
    await button(page, 'Send message').click();
    await expect(page.locator('.from-user').last()).toContainText('OLD_PROBLEM');
    await expect(button(page, 'Stop answer')).toHaveCount(0);
    await prompt(page).fill('OLD_DRAFT');
    await button(page, 'Multi Capture').click();
    await expect(page.locator('.chat-attachment')).toHaveCount(1);
    await startNew(page, action);
    await expect(roomPicker(page)).not.toHaveValue('');
    await expect(roomPicker(page)).not.toHaveValue(previous);
    const current = await roomPicker(page).inputValue();
    await expect(page.locator('.chat-message')).toHaveCount(0);
    await expect(page.locator('.chat-attachment')).toHaveCount(0);
    await expect(prompt(page)).toHaveValue('');
    await expect(button(page, 'Multi Capture')).toHaveAttribute('aria-pressed', 'false');
    await button(page, 'Capture').click();
    await expect(page.locator('.from-user')).toHaveCount(1);
    await expect(button(page, 'Stop answer')).toHaveCount(0);
    expect(submissions.at(-1)).toMatchObject({room_id:current, prompt:'', capture:true});
    expect(submissions.at(-1).file_ids ?? []).toEqual([]);
    const oldMessages = await (await request.get(`/api/rooms/${previous}/sync`)).json();
    expect(oldMessages.messages.some(m => m.text === 'OLD_PROBLEM')).toBe(true);
    if (action === 'New thought') {
      await startNew(page, action);
      await expect(roomPicker(page)).not.toHaveValue(current);
      await expect(roomPicker(page)).not.toHaveValue('');
      await expect(page.locator('.chat-message')).toHaveCount(0);
    }
  });
}

test('late snapshot and superseded room-creation responses cannot replace a fresh draft', async ({page}) => {
  const pending = new Map();
  const held = [];
  let holdPath;
  await page.routeWebSocket('**/api/ws', socket => {
    const server = socket.connectToServer();
    socket.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if (holdPath && message.path?.endsWith(holdPath) && message.method === 'POST') pending.set(message.id, holdPath);
      server.send(raw);
    });
    server.onMessage(raw => {
      const message = JSON.parse(String(raw));
      if (message.type === 'response' && pending.has(message.id)) {
        held.push(() => socket.send(raw));
        pending.delete(message.id);
      } else socket.send(raw);
    });
  });
  await page.goto('/');
  await expect(button(page, 'Capture')).toBeEnabled();
  holdPath = '/snapshot';
  await button(page, 'Multi Capture').click();
  await expect.poll(() => held.length).toBe(1);
  await button(page, 'New chat').click();
  await expect(button(page, 'Capture')).toBeEnabled();
  const room = await roomPicker(page).inputValue();
  await prompt(page).fill('Fresh draft');
  held.shift()();
  await expect(page.locator('.chat-attachment')).toHaveCount(0);
  await expect(prompt(page)).toHaveValue('Fresh draft');

  holdPath = '/rooms';
  await button(page, 'New chat').click();
  await expect.poll(() => held.length).toBe(1);
  await startNew(page, 'New thought');
  await expect(button(page, 'Capture')).toBeEnabled();
  const latest = await roomPicker(page).inputValue();
  expect(latest).not.toBe(room);
  await prompt(page).fill('Latest draft');
  held.shift()();
  await expect(prompt(page)).toHaveValue('Latest draft');
  await expect(roomPicker(page)).toHaveValue(latest);
  await expect(page).toHaveURL(/\/$/);
  await prompt(page).press('Enter');
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user')).toContainText('Latest draft');
});

test('Enter sends staged captures together and resets the toolbar for the next capture', async ({page}) => {
  await page.goto('/');
  await button(page, 'New chat').click();
  await expect(button(page, 'Capture')).toBeEnabled();
  await button(page, 'Multi Capture').click();
  await expect(page.locator('.chat-attachment')).toHaveCount(1);
  await button(page, 'Multi Capture').click();
  await expect(page.locator('.chat-attachment')).toHaveCount(2);
  await prompt(page).fill('Solve these together');
  await prompt(page).press('Enter');
  await expect(page.locator('.from-user')).toHaveCount(1);
  await expect(page.locator('.from-user .chat-photo')).toHaveCount(2);
  await expect(page.locator('.chat-attachment')).toHaveCount(0);
  await expect(button(page, 'Capture')).toBeEnabled();
});

for (const [action, startPath] of [['New thought', '/'], ['New chat', '/'], ['New chat', '/chat']]) {
  test(`a failed ${action} from ${startPath} stays empty instead of reopening the old context`, async ({page}) => {
    let failReset = false;
    let polls = 0;
    await page.routeWebSocket('**/api/ws', socket => {
      const server = socket.connectToServer();
      socket.onMessage(raw => {
        const message = JSON.parse(String(raw));
        if (message.path === '/api/rooms' && message.method === 'GET') polls++;
        if (failReset && message.method === 'POST' && (message.path === '/api/rooms' || message.path === '/api/rooms/thought')) {
          socket.send(JSON.stringify({type:'response', id:message.id, status:503, data:{error:'Reset temporarily unavailable'}}));
        } else server.send(raw);
      });
    });
    await page.goto(startPath);
    await expect(button(page, 'Capture')).toBeEnabled();
    const previous = await roomPicker(page).inputValue();
    await prompt(page).fill('Must not be restored after reset');
    failReset = true;
    await startNew(page, action);
    await expect(page.getByRole('alert')).toContainText('Reset temporarily unavailable');
    const before = polls;
    // Incoming SSE room changes must not restore a failed reset's old context.
    for (const title of ['Remote room one', 'Remote room two']) {
      expect((await page.request.post('/api/rooms', {data:{title}})).ok()).toBe(true);
      await expect.poll(() => polls).toBeGreaterThan(before);
    }
    await expect.poll(() => polls).toBeGreaterThan(before + 1);
    await expect(roomPicker(page)).toHaveValue('');
    await expect(page.getByRole('group', {name:'Message toolbar', exact:true})).toBeInViewport();
    await expect(button(page, 'Capture')).toBeDisabled();
    await expect(button(page, 'Multi Capture')).toBeDisabled();
    await expect(button(page, 'Send message')).toBeDisabled();
    await expect(page.locator('.chat-message')).toHaveCount(0);
    failReset = false;
    await startNew(page, action);
    await expect(button(page, 'Capture')).toBeEnabled();
    await expect(roomPicker(page)).not.toHaveValue(previous);
    await expect(prompt(page)).toHaveValue('');
  });
}

for (const path of ['/api/files', '/api/think']) {
  test(`late ${path} completion cannot attach files or clear the next problem's prompt`, async ({page}) => {
    let hold = false;
    const requests = new Set();
    const held = [];
    await page.routeWebSocket('**/api/ws', socket => {
      const server = socket.connectToServer();
      socket.onMessage(raw => {
        const message = JSON.parse(String(raw));
        if (hold && message.method === 'POST' && message.path === path) requests.add(message.id);
        server.send(raw);
      });
      server.onMessage(raw => {
        const message = JSON.parse(String(raw));
        if (requests.delete(message.id)) held.push(() => socket.send(raw));
        else socket.send(raw);
      });
    });
    await page.goto('/');
    await expect(button(page, 'Capture')).toBeEnabled();
    hold = true;
    if (path === '/api/files') {
      await page.getByLabel('Attach to chat', {exact:true}).setInputFiles({name:'old.txt', mimeType:'text/plain', buffer:Buffer.from('OLD_FILE')});
    } else {
      await prompt(page).fill('OLD_REQUEST');
      await button(page, 'Send message').click();
    }
    await expect.poll(() => held.length).toBe(1);
    await startNew(page, 'New thought');
    await expect(button(page, 'Capture')).toBeEnabled();
    const room = await roomPicker(page).inputValue();
    await prompt(page).fill('NEW_DRAFT');
    held.shift()();
    hold = false;
    await expect(prompt(page)).toHaveValue('NEW_DRAFT');
    await expect(page.locator('.chat-attachment, .chat-message')).toHaveCount(0);
    await expect(roomPicker(page)).toHaveValue(room);
    await prompt(page).press('Enter');
    await expect(page.locator('.from-user')).toHaveCount(1);
    await expect(page.locator('.from-user')).toContainText('NEW_DRAFT');
    await expect(page.locator('.chat-file')).toHaveCount(0);
  });
}

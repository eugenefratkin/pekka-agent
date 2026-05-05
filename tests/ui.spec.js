// @ts-check
const { test, expect } = require('@playwright/test');

// ─── Page load ────────────────────────────────────────────────────────────────

test('page loads with correct title and layout', async ({ page }) => {
  await page.goto('/');
  await expect(page).toHaveTitle('pekka-llm');

  // Logo
  await expect(page.locator('.logo')).toContainText('pekka');

  // Both panel headers present
  await expect(page.locator('text=Conversation')).toBeVisible();
  await expect(page.locator('text=Execution Traces')).toBeVisible();

  // Input is present but Send is disabled (no session yet)
  await expect(page.locator('#msgInput')).toBeVisible();
  await expect(page.locator('#btnSend')).toBeDisabled();
});

test('tab buttons are present and Agent Loop is active by default', async ({ page }) => {
  await page.goto('/');
  const tabs = page.locator('.tab');
  await expect(tabs).toHaveCount(2);
  await expect(tabs.first()).toHaveClass(/active/);
  await expect(tabs.first()).toContainText('Agent Loop');
});

test('status pill shows disconnected before session', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#sessionPill')).toContainText('no session');
});

// ─── Session creation ─────────────────────────────────────────────────────────

test('New session button creates a session and enables Send', async ({ page }) => {
  await page.goto('/');

  // Intercept the session API to avoid needing the real backend
  await page.route('/api/session', async (route) => {
    await route.fulfill({
      status: 200,
      contentType: 'application/json',
      body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
    });
  });

  await page.locator('#btnNew').click();

  // Session pill updated
  await expect(page.locator('#sessionPill')).toContainText('aaaabbbb');
  // Send button enabled
  await expect(page.locator('#btnSend')).toBeEnabled();
  // Confirmation message in history
  await expect(page.locator('.msg.sys')).toContainText('aaaabbbb');
});

test('session creation failure shows error message', async ({ page }) => {
  await page.goto('/');

  await page.route('/api/session', async (route) => {
    await route.fulfill({ status: 503, body: 'orchestrator unavailable' });
  });

  await page.locator('#btnNew').click();
  await expect(page.locator('.msg.err')).toBeVisible();
  await expect(page.locator('.msg.err')).toContainText('Could not create session');
});

// ─── Chat input ───────────────────────────────────────────────────────────────

test('pressing Enter sends the message', async ({ page }) => {
  await page.goto('/');

  // Set up session
  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));

  // Set up stream endpoint — return a minimal SSE final answer
  await page.route('/api/session/*/stream', (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: 'event: agent\ndata: {"type":"final_answer","content":"Hello!","iterations":1}\n\n',
  }));

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('hi');
  await page.locator('#msgInput').press('Enter');

  // User message appears
  await expect(page.locator('.msg.user')).toContainText('hi');
});

test('message input is cleared after sending', async ({ page }) => {
  await page.goto('/');

  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));
  await page.route('/api/session/*/stream', (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: 'event: agent\ndata: {"type":"final_answer","content":"Done","iterations":1}\n\n',
  }));

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('test message');
  await page.locator('#btnSend').click();

  await expect(page.locator('#msgInput')).toHaveValue('');
});

// ─── Streaming / agent events ────────────────────────────────────────────────

test('final answer appears in chat history', async ({ page }) => {
  await page.goto('/');

  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));
  await page.route('/api/session/*/stream', (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: [
      'event: agent\ndata: {"type":"iteration_start","iteration":0}\n\n',
      'event: agent\ndata: {"type":"think_start","iteration":0}\n\n',
      'event: agent\ndata: {"type":"think_done","iteration":0,"partial_text":null}\n\n',
      'event: agent\ndata: {"type":"final_answer","content":"42 is the answer","iterations":1}\n\n',
    ].join(''),
  }));

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('what is the answer');
  await page.locator('#btnSend').click();

  // Wait for bot message
  await expect(page.locator('.msg.bot')).toContainText('42 is the answer', { timeout: 10_000 });
});

test('agent loop panel opens and shows steps while running', async ({ page }) => {
  await page.goto('/');

  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));
  await page.route('/api/session/*/stream', (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: [
      'event: agent\ndata: {"type":"iteration_start","iteration":0}\n\n',
      'event: agent\ndata: {"type":"think_start","iteration":0}\n\n',
      'event: agent\ndata: {"type":"think_done","iteration":0,"partial_text":null}\n\n',
      'event: agent\ndata: {"type":"act_start","iteration":0,"num_tools":1}\n\n',
      'event: agent\ndata: {"type":"tool_call_start","call_id":"c1","name":"calculator","args":{"expression":"2+2"}}\n\n',
      'event: agent\ndata: {"type":"tool_call_done","call_id":"c1","name":"calculator","result":"4","success":true}\n\n',
      'event: agent\ndata: {"type":"observe_done","iteration":0}\n\n',
      'event: agent\ndata: {"type":"final_answer","content":"2+2=4","iterations":2}\n\n',
    ].join(''),
  }));

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('what is 2+2');
  await page.locator('#btnSend').click();

  // Loop panel should open
  await expect(page.locator('#loopPanel')).toHaveClass(/open/, { timeout: 5_000 });
  // Should show at least one step card
  await expect(page.locator('#loopBody .step')).toHaveCount({ min: 1 }, { timeout: 5_000 });
  // Final answer in chat
  await expect(page.locator('.msg.bot')).toContainText('2+2=4', { timeout: 10_000 });
});

test('error event shows error message in chat', async ({ page }) => {
  await page.goto('/');

  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));
  await page.route('/api/session/*/stream', (route) => route.fulfill({
    status: 200,
    contentType: 'text/event-stream',
    body: 'event: agent\ndata: {"type":"error","message":"LLM timeout"}\n\n',
  }));

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('hello');
  await page.locator('#btnSend').click();

  await expect(page.locator('.msg.err')).toBeVisible({ timeout: 5_000 });
});

// ─── Cancel ───────────────────────────────────────────────────────────────────

test('Cancel button calls DELETE /cancel endpoint', async ({ page }) => {
  await page.goto('/');

  let cancelCalled = false;

  await page.route('/api/session', (route) => route.fulfill({
    status: 200,
    contentType: 'application/json',
    body: JSON.stringify({ session_id: 'aaaabbbb-0000-0000-0000-000000000000' }),
  }));
  // Hang the stream so Cancel is needed
  await page.route('/api/session/*/stream', (route) => {
    // Don't fulfill — leave it hanging
  });
  await page.route('/api/session/*/cancel', async (route) => {
    cancelCalled = true;
    await route.fulfill({ status: 204 });
  });

  await page.locator('#btnNew').click();
  await page.locator('#msgInput').fill('hang');
  await page.locator('#btnSend').click();

  // Cancel button should become enabled while sending
  await expect(page.locator('#btnCancel')).toBeEnabled({ timeout: 5_000 });
  await page.locator('#btnCancel').click();

  // Small wait to let the fetch fire
  await page.waitForTimeout(300);
  expect(cancelCalled).toBe(true);
});

// ─── Trace panel ─────────────────────────────────────────────────────────────

test('trace panel is visible on load with placeholder text', async ({ page }) => {
  await page.goto('/');
  await expect(page.locator('#traceBody')).toBeVisible();
  await expect(page.locator('#traceBody')).toContainText('Spans will appear here');
});

test('Clear button empties the trace panel', async ({ page }) => {
  await page.goto('/');

  // Inject a fake span card via JS so we have something to clear
  await page.evaluate(() => {
    const body = document.getElementById('traceBody');
    body.innerHTML = '<div class="trace-card" data-tid="abc">fake</div>';
  });
  await expect(page.locator('.trace-card')).toBeVisible();

  await page.locator('#btnClear').click();
  await expect(page.locator('.trace-card')).not.toBeVisible();
});

// ─── Legend ───────────────────────────────────────────────────────────────────

test('trace legend shows all span types', async ({ page }) => {
  await page.goto('/');
  const expected = ['session', 'iteration', 'think', 'act', 'observe', 'tool.call'];
  const legend = page.locator('.legend');
  for (const label of expected) {
    await expect(legend).toContainText(label);
  }
});

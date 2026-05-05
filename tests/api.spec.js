// @ts-check
/**
 * API-level tests using Playwright's request context.
 * These hit the actual Rust server and require `cargo run --bin web` to be running.
 * Run with: npx playwright test tests/api.spec.js
 */
const { test, expect } = require('@playwright/test');

// ─── Session lifecycle ────────────────────────────────────────────────────────

test('POST /api/session returns a valid UUID', async ({ request }) => {
  const res = await request.post('/api/session');
  expect(res.status()).toBe(200);
  const body = await res.json();
  expect(body).toHaveProperty('session_id');
  expect(body.session_id).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/
  );
});

test('POST /api/session/:id/chat returns an answer string', async ({ request }) => {
  // Create session first
  const sesRes = await request.post('/api/session');
  const { session_id } = await sesRes.json();

  const chatRes = await request.post(`/api/session/${session_id}/chat`, {
    data: { message: 'Say hello in one word' },
  });
  expect(chatRes.status()).toBe(200);
  const body = await chatRes.json();
  expect(body).toHaveProperty('answer');
  expect(typeof body.answer).toBe('string');
  expect(body.answer.length).toBeGreaterThan(0);
});

test('POST /api/session/:id/chat with unknown session_id returns 404', async ({ request }) => {
  const res = await request.post('/api/session/00000000-0000-0000-0000-000000000000/chat', {
    data: { message: 'hello' },
  });
  expect(res.status()).toBe(404);
});

test('DELETE /api/session/:id/cancel returns 204', async ({ request }) => {
  const sesRes = await request.post('/api/session');
  const { session_id } = await sesRes.json();

  const cancelRes = await request.delete(`/api/session/${session_id}/cancel`);
  expect(cancelRes.status()).toBe(204);
});

// ─── Streaming endpoint ───────────────────────────────────────────────────────

test('POST /api/session/:id/stream returns SSE content-type', async ({ request }) => {
  const sesRes = await request.post('/api/session');
  const { session_id } = await sesRes.json();

  const streamRes = await request.post(`/api/session/${session_id}/stream`, {
    data: { message: 'ping' },
    timeout: 20_000,
  });
  expect(streamRes.status()).toBe(200);
  const ct = streamRes.headers()['content-type'] ?? '';
  expect(ct).toContain('text/event-stream');
});

test('SSE stream contains a final_answer event', async ({ request }) => {
  const sesRes = await request.post('/api/session');
  const { session_id } = await sesRes.json();

  const streamRes = await request.post(`/api/session/${session_id}/stream`, {
    data: { message: 'What is 1+1? Respond in one sentence.' },
    timeout: 30_000,
  });

  const text = await streamRes.text();
  // Parse all data lines
  const events = text
    .split('\n')
    .filter((l) => l.startsWith('data: '))
    .map((l) => { try { return JSON.parse(l.slice(6)); } catch { return null; } })
    .filter(Boolean);

  const finalAnswer = events.find((e) => e.type === 'final_answer');
  expect(finalAnswer).toBeDefined();
  expect(typeof finalAnswer.content).toBe('string');
  expect(finalAnswer.content.length).toBeGreaterThan(0);
});

// ─── OTel span stream ─────────────────────────────────────────────────────────

test('GET /events/spans returns SSE headers', async ({ request }) => {
  // We just verify the endpoint connects; we do not wait for actual spans
  const controller = new AbortController();
  const res = await request.get('/events/spans');
  // The SSE endpoint stays open; playwright request will return when it gets headers
  expect(res.status()).toBe(200);
  const ct = res.headers()['content-type'] ?? '';
  expect(ct).toContain('text/event-stream');
});

// ─── Static page ─────────────────────────────────────────────────────────────

test('GET / serves the HTML shell', async ({ request }) => {
  const res = await request.get('/');
  expect(res.status()).toBe(200);
  const ct = res.headers()['content-type'] ?? '';
  expect(ct).toContain('text/html');
  const html = await res.text();
  expect(html).toContain('pekka-llm');
});

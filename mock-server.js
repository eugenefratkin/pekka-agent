#!/usr/bin/env node
/**
 * pekka-llm mock server
 * Serves ui.html and stubs all API endpoints with realistic fake responses
 * so the full demo can be run without Rust installed.
 *
 * Usage:
 *   node mock-server.js          # starts on :3000
 *   PORT=4000 node mock-server.js
 */

'use strict';

const http   = require('http');
const fs     = require('fs');
const path   = require('path');
const crypto = require('crypto');

const PORT    = parseInt(process.env.PORT ?? '3000', 10);
const UI_HTML = path.join(__dirname, 'src', 'web', 'ui.html');

// ─── helpers ──────────────────────────────────────────────────────────────────

const sleep   = ms  => new Promise(r => setTimeout(r, ms));
const randHex = n   => crypto.randomBytes(n).toString('hex');
const traceId = ()  => randHex(16);   // 32 hex chars → OTel trace id
const spanId  = ()  => randHex(8);    // 16 hex chars → OTel span id

function jitter(base, spread = 0.3) {
  return Math.round(base * (1 + (Math.random() - 0.5) * spread));
}

// ─── SSE helpers ──────────────────────────────────────────────────────────────

function sseHeaders(res) {
  res.writeHead(200, {
    'Content-Type':  'text/event-stream',
    'Cache-Control': 'no-cache',
    'Connection':    'keep-alive',
    'X-Accel-Buffering': 'no',
  });
}

function sseEvent(res, name, data) {
  res.write(`event: ${name}\ndata: ${JSON.stringify(data)}\n\n`);
}

function agentEvent(res, data) {
  res.write(`event: agent\ndata: ${JSON.stringify(data)}\n\n`);
}

// ─── Span broadcast ───────────────────────────────────────────────────────────

const spanClients = new Set();

function addSpanClient(res) {
  spanClients.add(res);
  res.on('close', () => spanClients.delete(res));
}

function broadcastSpan(span) {
  const line = `event: span\ndata: ${JSON.stringify(span)}\n\n`;
  for (const client of spanClients) {
    try { client.write(line); } catch {}
  }
}

// ─── Span factories ───────────────────────────────────────────────────────────

function makeSpan(name, tid, sid, parentSid, startMs, durationMs, attrs = {}) {
  return {
    trace_id:       tid,
    span_id:        sid,
    parent_span_id: parentSid ?? '',
    name,
    start_ms:       startMs,
    end_ms:         startMs + durationMs,
    duration_ms:    durationMs,
    status:         'ok',
    attributes:     attrs,
  };
}

/**
 * Emit a realistic OTel trace for one request after the agent loop finishes.
 * Broadcasts each span individually so the waterfall builds up incrementally.
 */
async function emitTrace(sessionId, model, iterations, toolCalls) {
  const tid   = traceId();
  const now   = Date.now();
  const spans = [];

  // ── react.session (root) ──────────────────────────────────────────────────
  const sessionSid = spanId();
  const totalMs    = iterations * jitter(1800, 0.4);
  spans.push(makeSpan('react.session', tid, sessionSid, null, now, totalMs, {
    'react.session_id':    sessionId,
    'gen_ai.system':       'openai',
    'gen_ai.request.model': model,
  }));

  let offset = 20;

  for (let i = 0; i < iterations; i++) {
    const iterSid = spanId();
    const iterMs  = Math.round(totalMs / iterations);
    const iterStart = now + offset;
    spans.push(makeSpan('react.iteration', tid, iterSid, sessionSid, iterStart, iterMs - 10, {
      'react.session_id': sessionId,
      'react.iteration':  i,
    }));

    // react.think
    const thinkSid   = spanId();
    const thinkMs    = jitter(900, 0.35);
    const thinkStart = iterStart + 5;
    spans.push(makeSpan('react.think', tid, thinkSid, iterSid, thinkStart, thinkMs, {
      'gen_ai.operation.name':  'chat',
      'gen_ai.request.model':   model,
      'react.iteration':        i,
    }));

    let actOffset = thinkMs + 10;

    // react.act + tool.call spans (only iterations with tools)
    const iterTools = i < toolCalls.length ? toolCalls[i] : [];
    if (iterTools.length) {
      const actSid   = spanId();
      const actMs    = iterTools.reduce((s, t) => s + t.durationMs, 0) + 20;
      const actStart = iterStart + actOffset;
      spans.push(makeSpan('react.act', tid, actSid, iterSid, actStart, actMs, {
        'react.iteration':       i,
        'react.tool_call_count': iterTools.length,
      }));

      let toolOffset = 5;
      for (const tc of iterTools) {
        const toolSid   = spanId();
        const toolStart = actStart + toolOffset;
        spans.push(makeSpan('tool.call', tid, toolSid, actSid, toolStart, tc.durationMs, {
          'tool.name':    tc.name,
          'tool.call_id': tc.callId,
          'tool.success': true,
        }));
        toolOffset += tc.durationMs + 5;
      }

      actOffset += actMs + 10;

      // react.observe
      const obsSid   = spanId();
      const obsMs    = jitter(80, 0.4);
      spans.push(makeSpan('react.observe', tid, obsSid, iterSid, iterStart + actOffset, obsMs, {
        'react.iteration':  i,
        'react.num_results': iterTools.length,
      }));
    }

    offset += iterMs;
  }

  // Broadcast in creation order with tiny delays so the waterfall animates
  for (const span of spans) {
    broadcastSpan(span);
    await sleep(30);
  }
}

// ─── Demo scenarios ───────────────────────────────────────────────────────────

function pickScenario(message) {
  const m = message.toLowerCase();
  if (/\d[\d\s]*[+\-*/][\d\s]*\d|calculat|math|comput|add|subtract|multiply|divid/.test(m))
    return 'calculator';
  if (/weather|temperatur|forecast|rain/.test(m))
    return 'weather';
  if (/search|find|look\s*up|who is|what is/.test(m))
    return 'search';
  return 'direct';
}

// Evaluate simple arithmetic safely
function evalMath(expr) {
  try {
    // Only allow digits, spaces, operators, parens, dots
    if (!/^[\d\s+\-*/.()%]+$/.test(expr)) return null;
    // eslint-disable-next-line no-new-func
    const result = Function('"use strict"; return (' + expr + ')')();
    return typeof result === 'number' ? String(result) : null;
  } catch { return null; }
}

function extractExpression(message) {
  const m = message.match(/[\d][\d\s+\-*/.()%]*/);
  return m ? m[0].trim() : '2 + 2';
}

async function streamScenario(res, message, sessionId, model) {
  const scenario = pickScenario(message);

  if (scenario === 'direct') {
    // ── 1 iteration, no tools ──
    agentEvent(res, { type: 'iteration_start', iteration: 0 });
    await sleep(jitter(120));
    agentEvent(res, { type: 'think_start', iteration: 0 });
    await sleep(jitter(1100, 0.4));
    agentEvent(res, { type: 'think_done', iteration: 0, partial_text: null });
    await sleep(50);

    const answers = [
      `That's an interesting question. Based on my knowledge, I'd say: ${message.endsWith('?') ? message.replace(/\?$/, '') + '.' : 'it depends on the context.'} Would you like me to dig deeper into any particular aspect?`,
      `Great question! Here's my take: the key insight is that every system has trade-offs. For your specific case — "${message}" — I'd recommend starting simple and iterating based on feedback.`,
      `I've thought about this carefully. The short answer is: yes, with caveats. The long answer involves understanding the constraints, the goals, and the available options. Happy to elaborate on any of those.`,
    ];
    const answer = answers[Math.floor(Math.random() * answers.length)];

    agentEvent(res, { type: 'final_answer', content: answer, iterations: 1 });
    emitTrace(sessionId, model, 1, [[]]);
    res.end();
    return;
  }

  if (scenario === 'calculator') {
    // ── 2 iterations: think → call calculator → think → answer ──
    const expr   = extractExpression(message);
    const result = evalMath(expr) ?? '42';
    const callId = randHex(4);

    agentEvent(res, { type: 'iteration_start', iteration: 0 });
    await sleep(jitter(100));
    agentEvent(res, { type: 'think_start', iteration: 0 });
    await sleep(jitter(700, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 0, partial_text: 'I should use the calculator tool for this.' });
    await sleep(80);
    agentEvent(res, { type: 'act_start', iteration: 0, num_tools: 1 });
    await sleep(50);
    agentEvent(res, { type: 'tool_call_start', call_id: callId, name: 'calculator',
      args: { expression: expr } });
    await sleep(jitter(400, 0.4));
    agentEvent(res, { type: 'tool_call_done', call_id: callId, name: 'calculator',
      result, success: true });
    await sleep(60);
    agentEvent(res, { type: 'observe_done', iteration: 0 });
    await sleep(100);

    agentEvent(res, { type: 'iteration_start', iteration: 1 });
    await sleep(jitter(80));
    agentEvent(res, { type: 'think_start', iteration: 1 });
    await sleep(jitter(600, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 1, partial_text: null });
    await sleep(50);

    const answer = `The result of \`${expr}\` is **${result}**.`;
    agentEvent(res, { type: 'final_answer', content: answer, iterations: 2 });

    emitTrace(sessionId, model, 2, [
      [{ name: 'calculator', callId, durationMs: jitter(380) }],
      [],
    ]);
    res.end();
    return;
  }

  if (scenario === 'weather') {
    const callId = randHex(4);
    const city   = message.match(/in ([A-Za-z\s]+)/i)?.[1]?.trim() ?? 'San Francisco';
    const conditions = ['Sunny, 72°F', 'Partly cloudy, 65°F', 'Overcast, 58°F', 'Clear, 78°F'];
    const weather = conditions[Math.floor(Math.random() * conditions.length)];

    agentEvent(res, { type: 'iteration_start', iteration: 0 });
    await sleep(jitter(100));
    agentEvent(res, { type: 'think_start', iteration: 0 });
    await sleep(jitter(600, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 0, partial_text: `I'll fetch the weather for ${city}.` });
    await sleep(80);
    agentEvent(res, { type: 'act_start', iteration: 0, num_tools: 1 });
    await sleep(50);
    agentEvent(res, { type: 'tool_call_start', call_id: callId, name: 'fetch_weather',
      args: { city } });
    await sleep(jitter(550, 0.4));
    agentEvent(res, { type: 'tool_call_done', call_id: callId, name: 'fetch_weather',
      result: `{"city":"${city}","conditions":"${weather}"}`, success: true });
    await sleep(60);
    agentEvent(res, { type: 'observe_done', iteration: 0 });
    await sleep(100);

    agentEvent(res, { type: 'iteration_start', iteration: 1 });
    await sleep(jitter(80));
    agentEvent(res, { type: 'think_start', iteration: 1 });
    await sleep(jitter(500, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 1, partial_text: null });
    await sleep(50);

    const answer = `The current weather in **${city}** is: ${weather}.`;
    agentEvent(res, { type: 'final_answer', content: answer, iterations: 2 });

    emitTrace(sessionId, model, 2, [
      [{ name: 'fetch_weather', callId, durationMs: jitter(520) }],
      [],
    ]);
    res.end();
    return;
  }

  if (scenario === 'search') {
    // ── 3 iterations: think → search → think → maybe another tool → answer ──
    const callId1 = randHex(4);
    const callId2 = randHex(4);
    const query   = message.replace(/^(search|find|look up|who is|what is)\s*/i, '').replace(/\?$/, '');

    agentEvent(res, { type: 'iteration_start', iteration: 0 });
    await sleep(jitter(100));
    agentEvent(res, { type: 'think_start', iteration: 0 });
    await sleep(jitter(700, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 0, partial_text: `I should search for "${query}".` });
    await sleep(80);
    agentEvent(res, { type: 'act_start', iteration: 0, num_tools: 1 });
    await sleep(50);
    agentEvent(res, { type: 'tool_call_start', call_id: callId1, name: 'web_search',
      args: { query } });
    await sleep(jitter(700, 0.4));
    agentEvent(res, { type: 'tool_call_done', call_id: callId1, name: 'web_search',
      result: `Top results for "${query}": [1] Wikipedia article... [2] Official docs... [3] Recent news...`,
      success: true });
    await sleep(60);
    agentEvent(res, { type: 'observe_done', iteration: 0 });
    await sleep(100);

    agentEvent(res, { type: 'iteration_start', iteration: 1 });
    await sleep(jitter(80));
    agentEvent(res, { type: 'think_start', iteration: 1 });
    await sleep(jitter(800, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 1, partial_text: 'Let me fetch the first result for details.' });
    await sleep(80);
    agentEvent(res, { type: 'act_start', iteration: 1, num_tools: 1 });
    await sleep(50);
    agentEvent(res, { type: 'tool_call_start', call_id: callId2, name: 'fetch_url',
      args: { url: `https://example.com/search?q=${encodeURIComponent(query)}` } });
    await sleep(jitter(500, 0.4));
    agentEvent(res, { type: 'tool_call_done', call_id: callId2, name: 'fetch_url',
      result: `Page content about "${query}": This is a comprehensive overview covering history, usage, and examples.`,
      success: true });
    await sleep(60);
    agentEvent(res, { type: 'observe_done', iteration: 1 });
    await sleep(100);

    agentEvent(res, { type: 'iteration_start', iteration: 2 });
    await sleep(jitter(80));
    agentEvent(res, { type: 'think_start', iteration: 2 });
    await sleep(jitter(600, 0.3));
    agentEvent(res, { type: 'think_done', iteration: 2, partial_text: null });
    await sleep(50);

    const answer = `Here's what I found about **"${query}"**:\n\nBased on multiple sources, this is a well-documented topic with rich history and broad applications. The key points are:\n\n1. It originated as a foundational concept in the field.\n2. Modern usage has evolved significantly with new tooling.\n3. The community actively maintains documentation and examples.\n\nWould you like me to go deeper on any of these points?`;
    agentEvent(res, { type: 'final_answer', content: answer, iterations: 3 });

    emitTrace(sessionId, model, 3, [
      [{ name: 'web_search', callId: callId1, durationMs: jitter(680) }],
      [{ name: 'fetch_url',  callId: callId2, durationMs: jitter(480) }],
      [],
    ]);
    res.end();
  }
}

// ─── Request router ───────────────────────────────────────────────────────────

function parseBody(req) {
  return new Promise((resolve, reject) => {
    let data = '';
    req.on('data', c => data += c);
    req.on('end', () => {
      try { resolve(data ? JSON.parse(data) : {}); }
      catch (e) { reject(e); }
    });
    req.on('error', reject);
  });
}

function json(res, status, body) {
  const payload = JSON.stringify(body);
  res.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': Buffer.byteLength(payload) });
  res.end(payload);
}

const MODEL = 'gpt-4o-mini (mock)';

const server = http.createServer(async (req, res) => {
  const { method, url } = req;

  // ── GET / → serve ui.html ───────────────────────────────────────────────────
  if (method === 'GET' && (url === '/' || url === '/index.html')) {
    try {
      const html = fs.readFileSync(UI_HTML);
      res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8', 'Content-Length': html.length });
      res.end(html);
    } catch (e) {
      res.writeHead(500); res.end('Could not read ui.html: ' + e.message);
    }
    return;
  }

  // ── POST /api/session ───────────────────────────────────────────────────────
  if (method === 'POST' && url === '/api/session') {
    const session_id = crypto.randomUUID();
    console.log(`[session] created ${session_id}`);
    json(res, 200, { session_id });
    return;
  }

  // ── POST /api/session/:id/stream ────────────────────────────────────────────
  const streamMatch = url.match(/^\/api\/session\/([^/]+)\/stream$/);
  if (method === 'POST' && streamMatch) {
    const sessionId = streamMatch[1];
    let body;
    try { body = await parseBody(req); }
    catch { res.writeHead(400); res.end('bad json'); return; }

    const message = body.message ?? '';
    console.log(`[stream]  session=${sessionId.slice(0,8)} msg="${message.slice(0,60)}"`);

    sseHeaders(res);
    try {
      await streamScenario(res, message, sessionId, MODEL);
    } catch (e) {
      agentEvent(res, { type: 'error', message: e.message });
      res.end();
    }
    return;
  }

  // ── POST /api/session/:id/chat (non-streaming) ──────────────────────────────
  const chatMatch = url.match(/^\/api\/session\/([^/]+)\/chat$/);
  if (method === 'POST' && chatMatch) {
    let body;
    try { body = await parseBody(req); }
    catch { res.writeHead(400); res.end('bad json'); return; }
    json(res, 200, { answer: `Echo (mock): ${body.message ?? ''}` });
    return;
  }

  // ── DELETE /api/session/:id/cancel ──────────────────────────────────────────
  const cancelMatch = url.match(/^\/api\/session\/([^/]+)\/cancel$/);
  if (method === 'DELETE' && cancelMatch) {
    res.writeHead(204); res.end();
    return;
  }

  // ── GET /events/spans (SSE) ─────────────────────────────────────────────────
  if (method === 'GET' && url === '/events/spans') {
    sseHeaders(res);
    // Send a keepalive comment immediately
    res.write(': connected\n\n');
    addSpanClient(res);
    // Keep the connection alive with periodic comments
    const keepalive = setInterval(() => {
      try { res.write(': ping\n\n'); } catch { clearInterval(keepalive); }
    }, 15_000);
    req.on('close', () => clearInterval(keepalive));
    return;
  }

  // ── 404 ─────────────────────────────────────────────────────────────────────
  res.writeHead(404); res.end('Not found');
});

server.listen(PORT, () => {
  console.log(`\n  pekka-llm mock server\n`);
  console.log(`  ➜  http://localhost:${PORT}\n`);
  console.log(`  Try asking about: math, weather, or any search query\n`);
});

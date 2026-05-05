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

// Load .env if present (no dotenv dependency needed)
try {
  const envPath = path.join(__dirname, '.env');
  fs.readFileSync(envPath, 'utf8').split('\n').forEach(line => {
    const m = line.match(/^([^#=\s][^=]*)=(.*)$/);
    if (m && !process.env[m[1]]) process.env[m[1]] = m[2].trim().replace(/^['"]|['"]$/g, '');
  });
} catch { /* .env not required */ }

const PORT              = parseInt(process.env.PORT ?? '3754', 10);
const UI_HTML           = path.join(__dirname, 'src', 'web', 'ui.html');
const PERPLEXITY_KEY    = process.env.PERPLEXITY_API_KEY ?? '';
const PERPLEXITY_MODEL  = 'sonar';
const PERPLEXITY_URL    = 'https://api.perplexity.ai/chat/completions';

// ─── Auth config ──────────────────────────────────────────────────────────────

const GOOGLE_CLIENT_ID     = process.env.GOOGLE_CLIENT_ID     ?? '';
const GOOGLE_CLIENT_SECRET = process.env.GOOGLE_CLIENT_SECRET ?? '';
const BASE_URL             = (process.env.BASE_URL ?? `http://localhost:${PORT}`).replace(/\/$/, '');
// Auth is active when Google credentials are provided.
// On a deployed (https) BASE_URL without credentials the server refuses all requests
// rather than silently opening itself up.
const AUTH_ENABLED = !!(GOOGLE_CLIENT_ID && GOOGLE_CLIENT_SECRET);
const IS_HTTPS     = BASE_URL.startsWith('https://');

// Emails allowed to log in (lowercase).
// Fail-closed: if the list is empty AND auth is enabled, nobody gets in.
const WHITELIST = new Set(
  (process.env.WHITELISTED_EMAILS ?? '')
    .split(',').map(e => e.trim().toLowerCase()).filter(Boolean)
);

// ─── Session store ────────────────────────────────────────────────────────────

const SESSION_TTL_MS = 7 * 24 * 60 * 60 * 1000;
const sessionStore   = new Map(); // sid → { email, expires }

function parseCookies(req) {
  const out = {};
  for (const part of (req.headers.cookie ?? '').split(';')) {
    const idx = part.indexOf('=');
    if (idx < 0) continue;
    const k = part.slice(0, idx).trim();
    const v = part.slice(idx + 1).trim();
    try { out[k] = decodeURIComponent(v); } catch { out[k] = v; }
  }
  return out;
}

function getSession(req) {
  const sid = parseCookies(req)['pekka_sid'];
  if (!sid) return null;
  const s = sessionStore.get(sid);
  if (!s || s.expires < Date.now()) { sessionStore.delete(sid); return null; }
  return s;
}

function createSession(email) {
  const sid = randHex(32);
  sessionStore.set(sid, { email, expires: Date.now() + SESSION_TTL_MS });
  return sid;
}

function sessionCookieHeader(sid) {
  const secure = BASE_URL.startsWith('https') ? '; Secure' : '';
  return `pekka_sid=${sid}; HttpOnly; SameSite=Lax; Path=/${secure}; Max-Age=${SESSION_TTL_MS / 1000}`;
}

/** Returns the session or redirects to /login and returns null. */
function requireAuth(req, res) {
  // Dev bypass: only when running on plain http AND credentials are absent.
  if (!AUTH_ENABLED && !IS_HTTPS) return { email: 'dev' };

  // On https without credentials → misconfigured; refuse everything.
  if (!AUTH_ENABLED && IS_HTTPS) {
    res.writeHead(503, { 'Content-Type': 'text/plain' });
    res.end('Server misconfigured: GOOGLE_CLIENT_ID / GOOGLE_CLIENT_SECRET not set.');
    return null;
  }

  const session = getSession(req);
  if (session) return session;
  res.writeHead(302, { Location: `/login?next=${encodeURIComponent(req.url)}` });
  res.end();
  return null;
}

// ─── Google OAuth helpers ─────────────────────────────────────────────────────

function googleAuthUrl(state) {
  return 'https://accounts.google.com/o/oauth2/v2/auth?' + new URLSearchParams({
    client_id:     GOOGLE_CLIENT_ID,
    redirect_uri:  `${BASE_URL}/auth/callback`,
    response_type: 'code',
    scope:         'openid email profile',
    state,
    prompt:        'select_account',
  });
}

async function exchangeCodeForEmail(code) {
  const tokenRes = await fetch('https://oauth2.googleapis.com/token', {
    method:  'POST',
    headers: { 'Content-Type': 'application/x-www-form-urlencoded' },
    body:    new URLSearchParams({
      code,
      client_id:     GOOGLE_CLIENT_ID,
      client_secret: GOOGLE_CLIENT_SECRET,
      redirect_uri:  `${BASE_URL}/auth/callback`,
      grant_type:    'authorization_code',
    }).toString(),
  });
  if (!tokenRes.ok) throw new Error(`token exchange failed: ${tokenRes.status}`);
  const { id_token } = await tokenRes.json();

  const infoRes = await fetch(`https://oauth2.googleapis.com/tokeninfo?id_token=${id_token}`);
  if (!infoRes.ok) throw new Error(`tokeninfo failed: ${infoRes.status}`);
  const info = await infoRes.json();
  if (!info.email_verified) throw new Error('Google email not verified');
  return info.email.toLowerCase();
}

// ─── Auth pages ───────────────────────────────────────────────────────────────

function loginPage(error = '') {
  const errHtml = error
    ? `<p style="margin-top:16px;color:#f85149;font-size:12px">${esc(error)}</p>`
    : '';
  return `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>pekka-llm — Sign in</title>
<style>
*,*::before,*::after{box-sizing:border-box;margin:0;padding:0}
body{background:#0d1117;color:#e6edf3;min-height:100vh;display:flex;
     align-items:center;justify-content:center;
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}
.card{background:#161b22;border:1px solid #30363d;border-radius:12px;
      padding:40px;text-align:center;width:340px}
.logo{font-size:20px;font-weight:700;margin-bottom:6px}
.logo span{color:#7c3aed}
.sub{color:#7d8590;font-size:13px;margin-bottom:28px}
.google-btn{display:flex;align-items:center;justify-content:center;gap:10px;
  background:#fff;color:#1f1f1f;border:none;border-radius:6px;
  padding:11px 20px;font-size:14px;font-weight:500;cursor:pointer;
  text-decoration:none;width:100%;transition:opacity .15s}
.google-btn:hover{opacity:.9}
</style></head><body>
<div class="card">
  <div class="logo">◆ <span>pekka</span>-llm</div>
  <p class="sub">Sign in to continue</p>
  <a href="/auth/google" class="google-btn">
    <svg width="18" height="18" viewBox="0 0 24 24">
      <path fill="#4285F4" d="M22.56 12.25c0-.78-.07-1.53-.2-2.25H12v4.26h5.92c-.26 1.37-1.04 2.53-2.21 3.31v2.77h3.57c2.08-1.92 3.28-4.74 3.28-8.09z"/>
      <path fill="#34A853" d="M12 23c2.97 0 5.46-.98 7.28-2.66l-3.57-2.77c-.98.66-2.23 1.06-3.71 1.06-2.86 0-5.29-1.93-6.16-4.53H2.18v2.84C3.99 20.53 7.7 23 12 23z"/>
      <path fill="#FBBC05" d="M5.84 14.09c-.22-.66-.35-1.36-.35-2.09s.13-1.43.35-2.09V7.07H2.18C1.43 8.55 1 10.22 1 12s.43 3.45 1.18 4.93l2.85-2.22.81-.62z"/>
      <path fill="#EA4335" d="M12 5.38c1.62 0 3.06.56 4.21 1.64l3.15-3.15C17.45 2.09 14.97 1 12 1 7.7 1 3.99 3.47 2.18 7.07l3.66 2.84c.87-2.6 3.3-4.53 6.16-4.53z"/>
    </svg>
    Sign in with Google
  </a>
  ${errHtml}
</div></body></html>`;
}

function deniedPage(email) {
  return `<!DOCTYPE html><html lang="en"><head><meta charset="UTF-8">
<title>pekka-llm — Access denied</title>
<style>
*{box-sizing:border-box;margin:0;padding:0}
body{background:#0d1117;color:#e6edf3;min-height:100vh;display:flex;
     align-items:center;justify-content:center;
     font-family:-apple-system,BlinkMacSystemFont,'Segoe UI',sans-serif}
.card{background:#161b22;border:1px solid #30363d;border-radius:12px;
      padding:40px;text-align:center;width:380px}
.logo{font-size:20px;font-weight:700;margin-bottom:6px}
.logo span{color:#7c3aed}
h2{margin:20px 0 8px;font-size:16px}
p{color:#7d8590;font-size:13px;margin-bottom:24px}
.email{font-family:monospace;background:#21262d;padding:2px 8px;border-radius:4px;font-size:12px}
a{color:#58a6ff;font-size:13px}
</style></head><body>
<div class="card">
  <div class="logo">◆ <span>pekka</span>-llm</div>
  <h2>Access denied</h2>
  <p><span class="email">${esc(email)}</span> is not on the allow-list.</p>
  <a href="/logout">Sign in with a different account</a>
</div></body></html>`;
}

function esc(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// ─── helpers ──────────────────────────────────────────────────────────────────

const sleep   = ms  => new Promise(r => setTimeout(r, ms));
const randHex = n   => crypto.randomBytes(n).toString('hex');
const traceId = ()  => randHex(16);   // 32 hex chars → OTel trace id
const spanId  = ()  => randHex(8);    // 16 hex chars → OTel span id

function jitter(base, spread = 0.3) {
  return Math.round(base * (1 + (Math.random() - 0.5) * spread));
}

// ─── Perplexity search ────────────────────────────────────────────────────────

/**
 * Call Perplexity's chat completions API with `query`.
 * Returns { content, citations } on success, or throws on error.
 * Falls back to null if no API key is set (caller uses mock response).
 */
async function callPerplexity(query, systemPrompt = 'Be precise and concise.') {
  if (!PERPLEXITY_KEY) return null;

  const body = JSON.stringify({
    model: PERPLEXITY_MODEL,
    messages: [
      { role: 'system', content: systemPrompt },
      { role: 'user',   content: query },
    ],
  });

  const res = await fetch(PERPLEXITY_URL, {
    method: 'POST',
    headers: {
      'Authorization': `Bearer ${PERPLEXITY_KEY}`,
      'Content-Type':  'application/json',
    },
    body,
  });

  if (!res.ok) {
    const err = await res.text().catch(() => '');
    throw new Error(`Perplexity ${res.status}: ${err}`);
  }

  const data = await res.json();
  const content   = data.choices?.[0]?.message?.content ?? '<empty>';
  const citations = data.citations ?? [];
  return { content, citations };
}

// ─── Query decomposition ──────────────────────────────────────────────────────

/** Break a question into 2-3 parallel sub-queries based on type. */
function decomposeQuery(message) {
  const lower = message.toLowerCase();
  if (/\bvs\b|versus|compar|difference|tradeoff|pros.*con|better.*than|which.*should/.test(lower)) {
    return [
      `${message} — key advantages and strengths`,
      `${message} — disadvantages and limitations`,
      `${message} — practical recommendations`,
    ];
  }
  if (/\bhow\b.*\bwork|\bexplain\b|\bwhat is\b|\bwhat are\b|\barchitecture\b|\bhow do\b/.test(lower)) {
    return [
      `${message} — overview and core concepts`,
      `${message} — practical examples and use cases`,
      `${message} — best practices and pitfalls`,
    ];
  }
  if (/\bfuture\b|\btrend\b|\b2024\b|\b2025\b|\blatest\b|\bnew\b|\brecent\b/.test(lower)) {
    return [
      `${message} — current state`,
      `${message} — recent developments and trends`,
    ];
  }
  // Always do at least 2 parallel searches to demonstrate parallelism
  return [
    message,
    `${message} — related context and background`,
  ];
}

// ─── Sentiment detection ──────────────────────────────────────────────────────

const ANGER_SIGNALS = [
  [/\b(stupid|idiot|useless|terrible|awful|hate\b|sucks|wtf|dumb|worst|ridiculous|pathetic|trash|garbage)\b/i, 2],
  [/\b(fuck|shit|crap|damn)\b/i, 2],
  [/[A-Z]{6,}/, 1],
  [/!{2,}/, 1],
  [/\b(not working|doesn.t work|broken|failed|nothing works|waste of time)\b/i, 1],
  [/\b(frustrated|annoyed|angry|pissed|upset)\b/i, 1],
];

function detectSentiment(message) {
  let score = 0;
  for (const [pattern, weight] of ANGER_SIGNALS) {
    if (pattern.test(message)) score += weight;
  }
  if (score >= 2) return { sentiment: 'angry',      confidence: Math.min(0.55 + score * 0.1, 0.97) };
  if (score >= 1) return { sentiment: 'frustrated', confidence: 0.72 };
  return              { sentiment: 'neutral',    confidence: 0.95 };
}

function getSupportResponse(sentiment, message) {
  const pool = {
    angry: [
      `I can hear that you're really frustrated right now, and that's completely valid. These situations can feel incredibly draining.\n\nLet's slow down together. You don't have to figure everything out at once — I'm here to help however I can.\n\nWhat's weighing on you most right now?`,
      `Your frustration makes total sense — I'd feel the same in your position. Sometimes when things aren't working, the irritation just builds up.\n\nI want to actually help, not just pile on more things to try. Can you tell me what you were hoping to accomplish? Starting from what you really need often reveals a clearer path forward.`,
    ],
    frustrated: [
      `I can tell this has been challenging, and it's okay to feel frustrated. Being stuck is genuinely tough.\n\nWould it help to step back and look at the bigger picture together? Sometimes a fresh angle reveals options we hadn't thought of.\n\nWhat's your main concern right now — what would feel like a real win?`,
      `Feeling stuck is hard, and it's okay to acknowledge that. You're dealing with something that isn't easy.\n\nI'm here — let's work through this together. What part feels most overwhelming right now?`,
    ],
  };
  const arr = pool[sentiment] || pool.frustrated;
  return arr[Math.floor(Math.random() * arr.length)];
}

// ─── Session state ────────────────────────────────────────────────────────────

/** Tracks which sessions are in psychologist mode and how many turns since switch. */
const calmingState = new Map(); // sessionId → { mode: 'psychologist'|'research', turns: number }

/**
 * Conversation history per session — sent to Perplexity as context so it knows
 * what has already been discussed. Capped at 10 turns to stay within token limits.
 */
const conversationHistory = new Map(); // sessionId → [{ role, content }]

function getHistory(sessionId) {
  return conversationHistory.get(sessionId) ?? [];
}

function appendHistory(sessionId, role, content) {
  const hist = getHistory(sessionId);
  hist.push({ role, content: content.slice(0, 1200) }); // cap per-turn length
  if (hist.length > 20) hist.splice(0, hist.length - 20); // keep last 10 turns (20 messages)
  conversationHistory.set(sessionId, hist);
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
async function emitTrace(sessionId, model, iterations, toolCalls, message = '', reasonings = []) {
  const tid   = traceId();
  const now   = Date.now();
  const spans = [];

  // ── react.session (root) ──────────────────────────────────────────────────
  const sessionSid = spanId();
  const totalMs    = iterations * jitter(1800, 0.4);
  spans.push(makeSpan('react.session', tid, sessionSid, null, now, totalMs, {
    'react.session_id':     sessionId,
    'gen_ai.system':        'openai',
    'gen_ai.request.model': model,
    'react.user_message':   message,
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
    const thinkAttrs = {
      'gen_ai.operation.name':  'chat',
      'gen_ai.request.model':   model,
      'react.iteration':        i,
    };
    if (i === 0 && message)           thinkAttrs['gen_ai.input']     = message;
    if (reasonings[i] != null)        thinkAttrs['react.reasoning']  = reasonings[i];
    spans.push(makeSpan('react.think', tid, thinkSid, iterSid, thinkStart, thinkMs, thinkAttrs));

    let actOffset = thinkMs + 10;

    // react.act + tool.call spans (only iterations with tools)
    const iterTools = i < toolCalls.length ? toolCalls[i] : [];
    if (iterTools.length) {
      const actSid   = spanId();
      const isParallelAct0 = iterTools.some(t => t.parallel);
      const actMs    = isParallelAct0
        ? Math.max(...iterTools.map(t => t.durationMs)) + 20
        : iterTools.reduce((s, t) => s + t.durationMs, 0) + 20;
      const actStart = iterStart + actOffset;
      spans.push(makeSpan('react.act', tid, actSid, iterSid, actStart, actMs, {
        'react.iteration':       i,
        'react.tool_call_count': iterTools.length,
      }));

      if (isParallelAct0) {
        // All tools start nearly simultaneously — shows parallelism in waterfall
        for (const tc of iterTools) {
          const toolSid   = spanId();
          const toolStart = actStart + 5 + (tc.startOffset || 0);
          spans.push(makeSpan('tool.call', tid, toolSid, actSid, toolStart, tc.durationMs, {
            'tool.name':    tc.name,
            'tool.call_id': tc.callId,
            'tool.success': true,
          }));
        }
      } else {
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

// ─── Parallel research scenario ───────────────────────────────────────────────

async function streamScenario(res, message, sessionId, model) {
  const subQueries = decomposeQuery(message);
  const reasoning  = subQueries.length > 1
    ? `I'll research this with ${subQueries.length} parallel searches simultaneously`
    : `I'll use Perplexity to research: "${message}"`;

  // Build context from conversation history
  const hist = getHistory(sessionId);
  const histContext = hist.length
    ? 'Prior conversation context:\n' + hist.map(h => `${h.role}: ${h.content}`).join('\n') + '\n\n'
    : '';
  const systemPrompt = histContext
    ? `${histContext}Be precise and concise. Use the prior context above to give coherent follow-up answers.`
    : 'Be precise and concise.';

  // ── Iteration 0: think → parallel searches ───────────────────────────────
  agentEvent(res, { type: 'iteration_start', iteration: 0 });
  await sleep(jitter(100));
  agentEvent(res, { type: 'think_start', iteration: 0 });
  await sleep(jitter(400, 0.2));
  agentEvent(res, { type: 'think_done', iteration: 0, partial_text: reasoning });
  await sleep(80);

  agentEvent(res, { type: 'act_start', iteration: 0, num_tools: subQueries.length });
  await sleep(40);

  const callIds = subQueries.map(() => randHex(4));
  // Emit all tool_call_start events nearly simultaneously
  for (let i = 0; i < subQueries.length; i++) {
    agentEvent(res, { type: 'tool_call_start', call_id: callIds[i],
      name: 'perplexity_search', args: { query: subQueries[i] } });
    if (i < subQueries.length - 1) await sleep(15);
  }

  // ── Run all searches in parallel ─────────────────────────────────────────
  const t0 = Date.now();
  const rawResults = await Promise.all(
    subQueries.map(q => callPerplexity(q, systemPrompt).catch(e => {
      console.error('[perplexity]', e.message); return null;
    }))
  );
  const parallelDuration = Date.now() - t0;

  // Emit tool_call_done for each with small stagger (they all finished, show in order)
  const toolResults = [];
  for (let i = 0; i < subQueries.length; i++) {
    const pplx = rawResults[i];
    const result = pplx
      ? (pplx.citations?.length
          ? `${pplx.content}\n\nSources:\n${pplx.citations.map((u, j) => `[${j+1}] ${u}`).join('\n')}`
          : pplx.content)
      : `(Perplexity unavailable) No answer for: "${subQueries[i]}"`;
    agentEvent(res, { type: 'tool_call_done', call_id: callIds[i],
      name: 'perplexity_search', result, success: !!pplx });
    toolResults.push(result);
    await sleep(25);
  }

  await sleep(60);
  agentEvent(res, { type: 'observe_done', iteration: 0 });
  await sleep(100);

  // ── Iteration 1: synthesise ───────────────────────────────────────────────
  agentEvent(res, { type: 'iteration_start', iteration: 1 });
  await sleep(jitter(80));
  agentEvent(res, { type: 'think_start', iteration: 1 });
  await sleep(jitter(400, 0.3));
  const synthNote = subQueries.length > 1 ? 'Synthesising results from parallel searches…' : null;
  agentEvent(res, { type: 'think_done', iteration: 1, partial_text: synthNote });
  await sleep(50);

  const finalAnswer = subQueries.length > 1
    ? toolResults.map((r, i) => `### ${subQueries[i].split(' — ')[1] ?? subQueries[i]}\n\n${r}`).join('\n\n---\n\n')
    : toolResults[0];

  agentEvent(res, { type: 'final_answer', content: finalAnswer, iterations: 2 });

  // Persist to history
  appendHistory(sessionId, 'user', message);
  appendHistory(sessionId, 'assistant', finalAnswer.slice(0, 800));

  // Parallel tool spans for the OTel waterfall
  const toolSpans = subQueries.map((_, i) => ({
    name: 'perplexity_search',
    callId: callIds[i],
    durationMs: Math.round(parallelDuration * (0.8 + Math.random() * 0.4)),
    parallel: true,
    startOffset: i * 15,
  }));

  emitTrace(sessionId, model, 2, [toolSpans, []], message, [reasoning, synthNote]);
  res.end();
}

// ─── Psychologist scenario (triggered by sentiment observer) ──────────────────

async function streamPsychologist(res, message, sessionId, model, sentiment) {
  const pct = Math.round(sentiment.confidence * 100);

  // ── Observer agent runs first ─────────────────────────────────────────────
  agentEvent(res, { type: 'observer_start', agent: 'sentiment-monitor' });
  await sleep(200);
  agentEvent(res, { type: 'observer_done', agent: 'sentiment-monitor',
    sentiment: sentiment.sentiment, confidence: sentiment.confidence });
  await sleep(120);
  agentEvent(res, { type: 'observer_interrupt',
    reason: `${sentiment.sentiment} detected (${pct}% confidence)`,
    action: 'activating support mode' });
  await sleep(250);
  agentEvent(res, { type: 'mode_switch', from: 'research', to: 'psychologist' });
  await sleep(300);

  // ── Iteration 0: psychologist thinks and responds ─────────────────────────
  agentEvent(res, { type: 'iteration_start', iteration: 0 });
  await sleep(100);
  agentEvent(res, { type: 'think_start', iteration: 0 });
  await sleep(jitter(500, 0.2));
  const reasoning = `User seems ${sentiment.sentiment}. I should respond with empathy and understanding before addressing any technical content.`;
  agentEvent(res, { type: 'think_done', iteration: 0, partial_text: reasoning });
  await sleep(80);

  agentEvent(res, { type: 'act_start', iteration: 0, num_tools: 1 });
  const callId = randHex(4);
  agentEvent(res, { type: 'tool_call_start', call_id: callId,
    name: 'emotional_support', args: { sentiment: sentiment.sentiment, approach: 'empathy-first' } });

  const t0 = Date.now();
  const psychSystem =
    `You are a compassionate AI support counselor. The user seems ${sentiment.sentiment}. ` +
    `Respond with empathy and warmth. Validate their feelings without being patronizing. ` +
    `Keep your response to 2-3 paragraphs. Acknowledge what they said, offer understanding, ` +
    `and end with a gentle open question that invites them to share more. ` +
    `Do NOT lecture or give unsolicited advice.`;

  let response = null;
  try {
    const pplx = await callPerplexity(message, psychSystem);
    response = pplx?.content ?? null;
  } catch (e) {
    console.error('[psychologist]', e.message);
  }
  if (!response) response = getSupportResponse(sentiment.sentiment, message);
  const toolDuration = Date.now() - t0;

  agentEvent(res, { type: 'tool_call_done', call_id: callId,
    name: 'emotional_support', result: response, success: true });
  await sleep(60);
  agentEvent(res, { type: 'observe_done', iteration: 0 });
  await sleep(100);

  agentEvent(res, { type: 'iteration_start', iteration: 1 });
  await sleep(80);
  agentEvent(res, { type: 'think_start', iteration: 1 });
  await sleep(jitter(300, 0.2));
  agentEvent(res, { type: 'think_done', iteration: 1, partial_text: 'Formulating empathetic response…' });
  await sleep(50);

  agentEvent(res, { type: 'final_answer', content: response, iterations: 2, mode: 'psychologist' });

  // Persist to history
  appendHistory(sessionId, 'user', message);
  appendHistory(sessionId, 'assistant', response.slice(0, 800));

  emitTrace(sessionId, model, 2,
    [[{ name: 'emotional_support', callId, durationMs: toolDuration || jitter(400) }], []],
    message, [reasoning, 'Formulating empathetic response…']);
  res.end();
}

// ─── Calm-transition scenario: user calmed down, switch back to research ──────

async function streamCalmTransition(res, message, sessionId, model) {
  agentEvent(res, { type: 'observer_start', agent: 'sentiment-monitor' });
  await sleep(200);
  agentEvent(res, { type: 'observer_done', agent: 'sentiment-monitor',
    sentiment: 'neutral', confidence: 0.9 });
  await sleep(120);
  agentEvent(res, { type: 'observer_calm', message: 'User sentiment has stabilised — returning to research mode' });
  await sleep(300);
  agentEvent(res, { type: 'mode_switch', from: 'psychologist', to: 'research' });
  await sleep(200);
  await streamScenario(res, message, sessionId, model);
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
  const { method } = req;
  // Strip query string for routing
  const url      = req.url.split('?')[0];
  const fullUrl  = req.url;
  const query    = Object.fromEntries(new URLSearchParams(req.url.includes('?') ? req.url.split('?')[1] : ''));

  // ── GET /login ──────────────────────────────────────────────────────────────
  if (method === 'GET' && url === '/login') {
    if (!AUTH_ENABLED) { res.writeHead(302, { Location: '/' }); res.end(); return; }
    const page = loginPage();
    res.writeHead(200, { 'Content-Type': 'text/html; charset=utf-8' });
    res.end(page);
    return;
  }

  // ── GET /auth/google → redirect to Google consent ──────────────────────────
  if (method === 'GET' && url === '/auth/google') {
    if (!AUTH_ENABLED) { res.writeHead(302, { Location: '/' }); res.end(); return; }
    const state = randHex(16);
    // Sanitise: only allow same-origin relative paths (no // proto-relative, no http*)
    const rawNext = query.next ?? '/';
    const next    = /^\/[^/]/.test(rawNext) || rawNext === '/' ? rawNext : '/';
    res.writeHead(302, {
      Location:   googleAuthUrl(state),
      'Set-Cookie': [
        `pekka_state=${state}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600`,
        `pekka_next=${encodeURIComponent(next)}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600`,
      ],
    });
    res.end();
    return;
  }

  // ── GET /auth/callback → exchange code, verify email, set session ───────────
  if (method === 'GET' && url === '/auth/callback') {
    try {
      const cookies  = parseCookies(req);
      const expected = cookies['pekka_state'];
      const rawNext  = decodeURIComponent(cookies['pekka_next'] ?? '/');
      const next     = /^\/[^/]/.test(rawNext) || rawNext === '/' ? rawNext : '/';

      if (!expected || expected !== query.state) {
        const page = loginPage('Invalid or expired login attempt. Please try again.');
        res.writeHead(400, { 'Content-Type': 'text/html' }); res.end(page); return;
      }
      if (query.error) {
        const page = loginPage(`Google error: ${query.error}`);
        res.writeHead(400, { 'Content-Type': 'text/html' }); res.end(page); return;
      }

      const email = await exchangeCodeForEmail(query.code);

      // Fail-closed: deny if whitelist is empty OR email is not on it.
      if (!WHITELIST.has(email)) {
        console.log(`[auth] denied: ${email}`);
        res.writeHead(403, { 'Content-Type': 'text/html' }); res.end(deniedPage(email)); return;
      }

      const sid = createSession(email);
      console.log(`[auth] login: ${email}`);
      res.writeHead(302, {
        Location:   next,
        'Set-Cookie': [
          sessionCookieHeader(sid),
          'pekka_state=; HttpOnly; Path=/; Max-Age=0',
          'pekka_next=;  HttpOnly; Path=/; Max-Age=0',
        ],
      });
      res.end();
    } catch (e) {
      console.error('[auth] callback error:', e.message);
      const page = loginPage('Sign-in failed. Please try again.');
      res.writeHead(500, { 'Content-Type': 'text/html' }); res.end(page);
    }
    return;
  }

  // ── GET /logout ─────────────────────────────────────────────────────────────
  if (method === 'GET' && url === '/logout') {
    const sid = parseCookies(req)['pekka_sid'];
    if (sid) sessionStore.delete(sid);
    res.writeHead(302, {
      Location:   '/login',
      'Set-Cookie': 'pekka_sid=; HttpOnly; Path=/; Max-Age=0',
    });
    res.end();
    return;
  }

  // ── Auth gate — all routes below require a valid session ────────────────────
  const session = requireAuth(req, res);
  if (!session) return;

  // ── GET /api/me ─────────────────────────────────────────────────────────────
  if (method === 'GET' && url === '/api/me') {
    json(res, 200, { email: session.email });
    return;
  }

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
    console.log(`[session] created ${session_id} (user: ${session.email})`);
    json(res, 200, { session_id });
    return;
  }

  // ── POST /api/session/:id/stream ────────────────────────────────────────────
  const streamMatch = fullUrl.match(/^\/api\/session\/([^/?]+)\/stream/);
  if (method === 'POST' && streamMatch) {
    const sessionId = streamMatch[1];
    let body;
    try { body = await parseBody(req); }
    catch { res.writeHead(400); res.end('bad json'); return; }

    const message = body.message ?? '';
    console.log(`[stream]  session=${sessionId.slice(0,8)} msg="${message.slice(0,60)}"`);

    sseHeaders(res);
    try {
      const sentiment = detectSentiment(message);
      const state = calmingState.get(sessionId) ?? { mode: 'research', turns: 0 };

      if (sentiment.sentiment !== 'neutral') {
        // Angry/frustrated → psychologist takes over
        calmingState.set(sessionId, { mode: 'psychologist', turns: 0 });
        await streamPsychologist(res, message, sessionId, MODEL, sentiment);
      } else if (state.mode === 'psychologist') {
        // Was in psychologist mode, user now seems calm → transition back
        calmingState.set(sessionId, { mode: 'research', turns: 0 });
        await streamCalmTransition(res, message, sessionId, MODEL);
      } else {
        await streamScenario(res, message, sessionId, MODEL);
      }
    } catch (e) {
      agentEvent(res, { type: 'error', message: e.message });
      res.end();
    }
    return;
  }

  // ── POST /api/session/:id/chat (non-streaming) ──────────────────────────────
  const chatMatch = fullUrl.match(/^\/api\/session\/([^/?]+)\/chat/);
  if (method === 'POST' && chatMatch) {
    let body;
    try { body = await parseBody(req); }
    catch { res.writeHead(400); res.end('bad json'); return; }
    json(res, 200, { answer: `Echo (mock): ${body.message ?? ''}` });
    return;
  }

  // ── DELETE /api/session/:id/cancel ──────────────────────────────────────────
  const cancelMatch = fullUrl.match(/^\/api\/session\/([^/?]+)\/cancel/);
  if (method === 'DELETE' && cancelMatch) {
    res.writeHead(204); res.end();
    return;
  }

  // ── GET /events/spans (SSE) ─────────────────────────────────────────────────
  if (method === 'GET' && url === '/events/spans') {
    sseHeaders(res);
    res.write(': connected\n\n');
    addSpanClient(res);
    const keepalive = setInterval(() => {
      try { res.write(': ping\n\n'); } catch { clearInterval(keepalive); }
    }, 15_000);
    req.on('close', () => clearInterval(keepalive));
    return;
  }

  // ── 404 ─────────────────────────────────────────────────────────────────────
  res.writeHead(404); res.end('Not found');
});

server.listen(PORT, '0.0.0.0', () => {
  console.log(`\n  pekka-llm mock server\n`);
  console.log(`  ➜  http://localhost:${PORT}\n`);
  console.log(`  Features:`);
  console.log(`    • Parallel Perplexity searches (2-3 sub-queries simultaneously)`);
  console.log(`    • Sentiment observer agent (anger → psychologist mode)`);
  console.log(`    • Session conversation history (Perplexity sees prior context)`);
  if (process.env.PERPLEXITY_API_KEY) {
    console.log(`\n  Perplexity: ENABLED (model: ${PERPLEXITY_MODEL})`);
  } else {
    console.log(`\n  Perplexity: DISABLED (set PERPLEXITY_API_KEY to enable)`);
  }
  console.log();
});

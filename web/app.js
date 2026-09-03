import { costUsd, PRICING_DATE, PROVIDER_LABEL } from './pricing.js';

const firebaseConfig = {
  apiKey: 'AIzaSyDWc8AdeuuvYPjY0i12TajgsY5uJjKGZmQ',
  authDomain: 'claude-usage-collector-fm.firebaseapp.com',
  projectId: 'claude-usage-collector-fm',
  appId: '1:410525568072:web:0270f380e5e2fe5fc09c7d',
};
firebase.initializeApp(firebaseConfig);
const auth = firebase.auth();
const db = firebase.firestore();

const $ = (id) => document.getElementById(id);
const fmtUsd = (v) => v == null ? '?' : new Intl.NumberFormat('en-US', { style: 'currency', currency: 'USD', maximumFractionDigits: v >= 100 ? 0 : 2 }).format(v);
const fmtTok = (n) => n >= 1e9 ? (n / 1e9).toFixed(2) + ' B' : n >= 1e6 ? (n / 1e6).toFixed(1) + ' M' : n >= 1e3 ? (n / 1e3).toFixed(0) + ' k' : String(n);
const fmtInt = (n) => new Intl.NumberFormat('en-US').format(n);
const pad = (n) => String(n).padStart(2, '0');
const iso = (d) => `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
const addDays = (d, n) => { const x = new Date(d); x.setDate(x.getDate() + n); return x; };
const ago = (ts) => {
  if (!ts) return 'never';
  const s = Math.max(0, (Date.now() - ts.toDate().getTime()) / 1000);
  if (s < 90) return `${Math.round(s)} s ago`;
  if (s < 5400) return `${Math.round(s / 60)} min ago`;
  if (s < 172800) return `${Math.round(s / 3600)} h ago`;
  return `${Math.round(s / 86400)} d ago`;
};

// ---- ranges ---------------------------------------------------------------

function buildRanges() {
  const now = new Date();
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const out = [];
  for (let i = 0; i < 6; i++) {
    const start = new Date(now.getFullYear(), now.getMonth() - i, 1);
    const end = new Date(now.getFullYear(), now.getMonth() - i + 1, 0);
    out.push({
      key: `m${i}`,
      label: start.toLocaleDateString('en-US', { month: 'long', year: 'numeric' }),
      from: iso(start), to: iso(end), monthly: true,
    });
  }
  out.splice(1, 0,
    { key: 'd30', label: 'Last 30 days', from: iso(addDays(today, -29)), to: iso(today), monthly: false },
    { key: 'd7', label: 'Last 7 days', from: iso(addDays(today, -6)), to: iso(today), monthly: false },
  );
  return out;
}
const RANGES = buildRanges();
function currentRange() {
  return RANGES.find((r) => r.key === $('range').value) || RANGES[0];
}

// ---- data -----------------------------------------------------------------

const EMPTY = () => ({ input: 0, output: 0, cache_read: 0, cache_write_5m: 0, cache_write_1h: 0, replies: 0 });
function addTotals(into, model, t) {
  const x = (into[model] ??= EMPTY());
  for (const k of Object.keys(x)) x[k] += Number(t[k] ?? 0);
}
function sumModels(models) {
  const t = EMPTY();
  for (const m of Object.values(models)) for (const k of Object.keys(t)) t[k] += m[k];
  return t;
}
function priceModels(models) {
  let usd = 0; const unknown = [];
  for (const [m, t] of Object.entries(models)) {
    const c = costUsd(m, t);
    if (c == null) unknown.push(m); else usd += c;
  }
  return { usd, unknown };
}
const cacheHit = (t) => {
  const prompt = t.input + t.cache_read + t.cache_write_5m + t.cache_write_1h;
  return prompt ? t.cache_read / prompt : null;
};

async function loadRange(range) {
  const users = await db.collection('users').get();
  const out = [];
  for (const u of users.docs) {
    const days = await u.ref.collection('days').where('date', '>=', range.from).where('date', '<=', range.to).get();
    const perDay = {};       // date -> models
    const perAccount = {};   // "host/label" -> { models, days:Set }
    const all = {};
    for (const d of days.docs) {
      const { date, host, account, provider, models } = d.data();
      const key = `${host}/${account}`;
      const acc = (perAccount[key] ??= { models: {}, days: new Set(), provider: provider ?? 'claude' });
      acc.days.add(date);
      for (const [model, t] of Object.entries(models ?? {})) {
        addTotals((perDay[date] ??= {}), model, t);
        addTotals(acc.models, model, t);
        addTotals(all, model, t);
      }
    }
    const data = u.data();
    out.push({ uid: u.id, displayName: data.displayName || data.email || u.id, accounts: data.accounts ?? {}, perDay, perAccount, all });
  }
  out.sort((a, b) => a.displayName.localeCompare(b.displayName));
  return out;
}

/** Subscription USD for a user in the range. Accounts are deduplicated on
 *  label+tier (same login on two machines is one subscription). Non-month
 *  ranges are pro-rated by days/30. */
function subscriptionUsd(u, range) {
  const seen = new Map();
  const daysInRange = (new Date(range.to) - new Date(range.from)) / 86400000 + 1;
  for (const [key, a] of Object.entries(u.accounts)) {
    const active = u.perAccount[key]?.days.size > 0 || (a.lastPush && iso(a.lastPush.toDate()) >= range.from && iso(a.lastPush.toDate()) <= range.to);
    if (!active) continue;
    const label = key.split('/')[1];
    const id = `${label}:${a.tier ?? ''}`;
    if (!seen.has(id) || (seen.get(id) == null && a.subscriptionUsd != null)) seen.set(id, a.subscriptionUsd ?? null);
  }
  let usd = 0, unknown = false;
  for (const v of seen.values()) { if (v == null) unknown = true; else usd += v; }
  if (!range.monthly) usd = usd * daysInRange / 30;
  return { usd, unknown, count: seen.size };
}

// ---- render ---------------------------------------------------------------

let chart;
function render(users, range) {
  const all = {};
  let api = 0, paid = 0, paidUnknown = false, unknownModels = new Set();
  for (const u of users) {
    for (const [m, t] of Object.entries(u.all)) addTotals(all, m, t);
    const p = priceModels(u.all); api += p.usd; p.unknown.forEach((m) => unknownModels.add(m));
    const s = subscriptionUsd(u, range); paid += s.usd; paidUnknown ||= s.unknown;
  }
  const total = sumModels(all);
  const saved = api - paid;
  const ratio = paid > 0 ? api / paid : null;

  $('c-api').textContent = fmtUsd(api);
  $('c-api-sub').textContent = `${fmtInt(total.replies)} replies · ${fmtTok(total.input + total.output + total.cache_read + total.cache_write_5m + total.cache_write_1h)} tokens`;
  $('c-paid').textContent = fmtUsd(paid) + (paidUnknown ? ' + ?' : '');
  $('c-paid-sub').textContent = range.monthly ? 'monthly subscriptions' : 'pro rata, days / 30';
  $('c-saved').textContent = (saved >= 0 ? '' : '−') + fmtUsd(Math.abs(saved)) + (ratio ? ` · ${ratio.toFixed(1)}×` : '');
  $('c-saved-sub').textContent = saved >= 0 ? 'API would have cost more' : 'API would have been cheaper';
  $('c-saved-card').className = 'card ' + (saved >= 0 ? 'good' : 'bad');
  const hit = cacheHit(total);
  $('c-cache').textContent = hit == null ? '–' : `${Math.round(hit * 100)} %`;
  $('c-cache-sub').textContent = `${fmtTok(total.cache_read)} read from cache`;

  try { renderChart(users, range, paid); } catch (e) { console.error('chart', e); }
  renderPeople(users, range);
  renderModels(all);
  $('footer').textContent = `Prices: Anthropic / OpenAI / Google list prices as of ${PRICING_DATE}. Unknown models shown as "?"` +
    (unknownModels.size ? ` (${[...unknownModels].join(', ')})` : '') + '. Days are local dates on the machine that collected them.';
}

function renderChart(users, range, paid) {
  const labels = [];
  for (let d = new Date(range.from); iso(d) <= range.to; d = addDays(d, 1)) labels.push(iso(d));
  const palette = ['#7aa2f7', '#9ece6a', '#e0af68', '#f7768e', '#bb9af7', '#7dcfff'];
  const datasets = users.map((u, i) => ({
    label: u.displayName,
    data: labels.map((d) => u.perDay[d] ? priceModels(u.perDay[d]).usd : 0),
    backgroundColor: palette[i % palette.length],
    stack: 'cost',
  }));
  const perDay = paid / labels.length;
  if (perDay > 0) datasets.push({ type: 'line', label: 'Subscription / day', data: labels.map(() => perDay), borderColor: '#8b93a5', borderDash: [4, 4], pointRadius: 0, borderWidth: 1 });
  const css = getComputedStyle(document.documentElement);
  const cfg = {
    type: 'bar',
    data: { labels: labels.map((l) => l.slice(5)), datasets },
    options: {
      responsive: true, maintainAspectRatio: false, animation: false,
      scales: {
        x: { stacked: true, grid: { display: false }, ticks: { color: css.getPropertyValue('--muted') } },
        y: { stacked: true, grid: { color: css.getPropertyValue('--border') }, ticks: { color: css.getPropertyValue('--muted'), callback: (v) => '$' + v } },
      },
      plugins: { legend: { labels: { color: css.getPropertyValue('--fg') } }, tooltip: { callbacks: { label: (c) => `${c.dataset.label}: ${fmtUsd(c.parsed.y)}` } } },
    },
  };
  if (chart) { chart.data = cfg.data; chart.options = cfg.options; chart.update(); } else chart = new Chart($('chart'), cfg);
}

function renderPeople(users, range) {
  const rows = [`<tr><th class="l">Person</th><th class="l">Account</th><th class="l">Machine</th><th class="l">Last push</th><th>Replies</th><th>Tokens</th><th>Cache hit</th><th>API-equivalent</th><th>Subscription</th><th>Ratio</th></tr>`];
  let tApi = 0, tPaid = 0, tRep = 0, tTok = 0;
  for (const u of users) {
    const p = priceModels(u.all); const s = subscriptionUsd(u, range); const t = sumModels(u.all);
    const tok = t.input + t.output + t.cache_read + t.cache_write_5m + t.cache_write_1h;
    tApi += p.usd; tPaid += s.usd; tRep += t.replies; tTok += tok;
    const hit = cacheHit(t);
    rows.push(`<tr><td class="l"><strong>${esc(u.displayName)}</strong></td><td class="l muted">${s.count} account${s.count === 1 ? '' : 's'}</td><td></td><td></td><td>${fmtInt(t.replies)}</td><td>${fmtTok(tok)}</td><td>${hit == null ? '–' : Math.round(hit * 100) + ' %'}</td><td>${fmtUsd(p.usd)}</td><td>${fmtUsd(s.usd)}${s.unknown ? ' + ?' : ''}</td><td>${s.usd > 0 ? (p.usd / s.usd).toFixed(1) + '×' : '–'}</td></tr>`);
    const keys = new Set([...Object.keys(u.accounts), ...Object.keys(u.perAccount)]);
    for (const key of [...keys].sort()) {
      const a = u.accounts[key] ?? {}; const pa = u.perAccount[key];
      if (!pa && !(a.lastPush && iso(a.lastPush.toDate()) >= range.from)) continue;
      const [host, label] = key.split('/');
      const models = pa?.models ?? {}; const pp = priceModels(models); const tt = sumModels(models);
      const ttok = tt.input + tt.output + tt.cache_read + tt.cache_write_5m + tt.cache_write_1h;
      const provider = a.provider ?? pa?.provider ?? 'claude';
      const name = a.display || label;
      const tierTxt = a.tier ? `${a.subscription ?? ''} ${a.tier.replace(/^default_claude_/, '')}`.trim() : '';
      const tier = `<span class="badge">${esc(PROVIDER_LABEL[provider] ?? provider)}${tierTxt ? ' · ' + esc(tierTxt) : ''}</span>`;
      const sub = a.subscriptionUsd == null ? '?' : fmtUsd(range.monthly ? a.subscriptionUsd : a.subscriptionUsd * ((new Date(range.to) - new Date(range.from)) / 86400000 + 1) / 30);
      rows.push(`<tr class="sub"><td></td><td class="l">${esc(name)} ${tier}</td><td class="l mono">${esc(host)}</td><td class="l">${ago(a.lastPush)}</td><td>${fmtInt(tt.replies)}</td><td>${fmtTok(ttok)}</td><td>${cacheHit(tt) == null ? '–' : Math.round(cacheHit(tt) * 100) + ' %'}</td><td>${fmtUsd(pp.usd)}</td><td>${sub}</td><td></td></tr>`);
    }
  }
  rows.push(`<tr class="total"><td class="l">Total</td><td></td><td></td><td></td><td>${fmtInt(tRep)}</td><td>${fmtTok(tTok)}</td><td></td><td>${fmtUsd(tApi)}</td><td>${fmtUsd(tPaid)}</td><td>${tPaid > 0 ? (tApi / tPaid).toFixed(1) + '×' : '–'}</td></tr>`);
  $('people').innerHTML = rows.join('');
}

const modelVendor = (m) => m.startsWith('claude-') ? 'Anthropic' : m.startsWith('gpt-') || m.startsWith('o') && /^o\d/.test(m) ? 'OpenAI' : m.startsWith('gemini-') ? 'Google' : '';
function renderModels(all) {
  const rows = [`<tr><th class="l">Model</th><th class="l">Vendor</th><th>Replies</th><th>Input</th><th>Output</th><th>Cache read</th><th>Cache write 5m</th><th>Cache write 1h</th><th>API-equivalent</th></tr>`];
  const entries = Object.entries(all).map(([m, t]) => [m, t, costUsd(m, t)]).sort((a, b) => (b[2] ?? -1) - (a[2] ?? -1));
  for (const [m, t, c] of entries) {
    rows.push(`<tr><td class="l mono">${esc(m)}</td><td class="l muted">${modelVendor(m)}</td><td>${fmtInt(t.replies)}</td><td>${fmtTok(t.input)}</td><td>${fmtTok(t.output)}</td><td>${fmtTok(t.cache_read)}</td><td>${fmtTok(t.cache_write_5m)}</td><td>${fmtTok(t.cache_write_1h)}</td><td>${c == null ? '?' : fmtUsd(c)}</td></tr>`);
  }
  $('models').innerHTML = rows.join('');
}

const esc = (s) => String(s).replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' }[c]));

// ---- wiring ---------------------------------------------------------------

let cache = new Map();
async function refresh(force = false) {
  const range = currentRange();
  $('refresh').disabled = true;
  try {
    if (force) cache = new Map();
    let users = cache.get(range.key);
    if (!users) { users = await loadRange(range); cache.set(range.key, users); }
    render(users, range);
  } catch (e) {
    console.error(e);
    $('footer').textContent = `Error: ${e.message}`;
  } finally {
    $('refresh').disabled = false;
  }
}

$('range').innerHTML = RANGES.map((r) => `<option value="${r.key}">${r.label}</option>`).join('');
try { const k = localStorage.getItem('range'); if (RANGES.some((r) => r.key === k)) $('range').value = k; } catch {}
$('range').addEventListener('change', () => { try { localStorage.setItem('range', $('range').value); } catch {} refresh(); });
$('refresh').addEventListener('click', () => refresh(true));
$('logout').addEventListener('click', () => auth.signOut());
$('login-form').addEventListener('submit', async (e) => {
  e.preventDefault();
  $('login-error').textContent = '';
  try { await auth.signInWithEmailAndPassword($('email').value, $('password').value); }
  catch (err) { $('login-error').textContent = err.message; }
});
auth.onAuthStateChanged((user) => {
  $('login').hidden = !!user;
  $('app').hidden = !user;
  if (user) refresh(true); else cache = new Map();
});

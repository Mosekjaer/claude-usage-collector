// API-equivalent pricing, USD per million tokens. Source: Anthropic price list
// as of 2026-06-24. Mirror of cosmic-ext-applet-claude-usage/src/pricing.rs.
// Longest matching model-id prefix wins.
export const PRICING_DATE = '2026-06-24';

const p = (input, output, cacheRead, cacheWrite5m, cacheWrite1h) => ({ input, output, cache_read: cacheRead, cache_write_5m: cacheWrite5m, cache_write_1h: cacheWrite1h });

export const TABLE = [
  ['claude-fable-5-1', p(10.0, 50.0, 0.25, 12.5, 20.0)],
  ['claude-mythos-5-1', p(10.0, 50.0, 0.25, 12.5, 20.0)],
  ['claude-fable-5', p(10.0, 50.0, 1.0, 12.5, 20.0)],
  ['claude-mythos-5', p(10.0, 50.0, 1.0, 12.5, 20.0)],
  ['claude-opus-5', p(5.0, 25.0, 0.5, 6.25, 10.0)],
  ['claude-opus-4-8', p(5.0, 25.0, 0.5, 6.25, 10.0)],
  ['claude-opus-4-7', p(5.0, 25.0, 0.5, 6.25, 10.0)],
  ['claude-opus-4-6', p(5.0, 25.0, 0.5, 6.25, 10.0)],
  ['claude-sonnet-5', p(2.0, 10.0, 0.2, 2.5, 4.0)],
  ['claude-sonnet-4-6', p(3.0, 15.0, 0.3, 3.75, 6.0)],
  ['claude-haiku-4-5', p(1.0, 5.0, 0.1, 1.25, 2.0)],
];

export function lookup(model) {
  let best = null;
  for (const [prefix, price] of TABLE) {
    if (model.startsWith(prefix) && (!best || prefix.length > best[0].length)) best = [prefix, price];
  }
  return best ? best[1] : null;
}

/** USD for one model's totals, or null when the model is unpriced. */
export function costUsd(model, t) {
  const pr = lookup(model);
  if (!pr) return null;
  const m = 1e6;
  return (t.input * pr.input + t.output * pr.output + t.cache_read * pr.cache_read
    + t.cache_write_5m * pr.cache_write_5m + t.cache_write_1h * pr.cache_write_1h) / m;
}

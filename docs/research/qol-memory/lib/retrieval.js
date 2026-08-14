export function tokens(text) {
  return (text.toLowerCase().match(/[\p{L}\p{N}]+/gu) || [])
    .filter((t) => t.length > 1)
    .map(normalize);
}

export function normalize(t) {
  if (t.length <= 3) return t;
  if (t.endsWith("ies") && t.length > 4) return t.slice(0, -3) + "y";
  if (t.endsWith("es") && t.length > 4 && (t.endsWith("ses") || t.endsWith("xes") || t.endsWith("zes") || t.endsWith("ches") || t.endsWith("shes"))) {
    return t.slice(0, -2);
  }
  if (t.endsWith("ss")) return t;
  if (t.endsWith("s") && t.length > 3) return t.slice(0, -1);
  if (t.endsWith("ing") && t.length > 6) return t.slice(0, -3);
  if (t.endsWith("ed") && t.length > 5) return t.slice(0, -2);
  if (t.endsWith("ly") && t.length > 5) return t.slice(0, -2);
  return t;
}

export function buildIndex(units) {
  const df = new Map();
  const docs = units.map((u) => {
    const tf = new Map();
    for (const t of tokens(u.text)) tf.set(t, (tf.get(t) || 0) + 1);
    for (const t of tf.keys()) df.set(t, (df.get(t) || 0) + 1);
    return { unit: u, tf, len: u.text.length };
  });
  const N = docs.length;
  const avgdl = docs.reduce((s, d) => s + d.len, 0) / Math.max(1, N);
  const idf = new Map();
  for (const [t, n] of df) idf.set(t, Math.log(1 + (N - n + 0.5) / (n + 0.5)));
  return { docs, idf, N, avgdl };
}

export function bm25Ranks(query, idx, weights, k) {
  const qt = tokens(query);
  if (!qt.length) return [];
  const scored = [];
  for (const d of idx.docs) {
    let s = 0;
    for (const t of qt) {
      const f = d.tf.get(t) || 0;
      if (!f) continue;
      const w = idx.idf.get(t) || 0;
      const boost = weights && weights[t] ? weights[t] : 1;
      s += (w * f * boost * 1.2) / (f + 1.2 * (1 - 0.75 + 0.75 * (d.len / idx.avgdl)));
    }
    scored.push([d.unit.key, s]);
  }
  scored.sort((a, b) => b[1] - a[1] || (a[0] < b[0] ? -1 : 1));
  const out = scored.map(([key, s]) => ({ key, score: s }));
  return k && k > 0 ? out.slice(0, k) : out;
}

export function snippet(text, matchWords, window = 240) {
  const lower = text.toLowerCase();
  let idx = -1;
  for (const w of matchWords) {
    const i = lower.indexOf(w);
    if (i >= 0) {
      idx = Math.min(idx === -1 ? i : idx, i);
    }
  }
  if (idx < 0) return text.slice(0, window);
  const start = Math.max(0, idx - Math.floor(window / 3));
  let s = text.slice(start, start + window).replace(/\s+/g, " ").trim();
  if (start > 0) s = "..." + s;
  if (start + window < text.length) s = s + "...";
  return s;
}

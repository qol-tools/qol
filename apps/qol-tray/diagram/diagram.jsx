// Diagram for the QoL Tray Runtime Architecture Map (top-down).
// Vertical bands stacked on a 1320 × 1960 canvas. Stage scales to fit the
// viewport WIDTH (capped at 1×) and scrolls vertically — no letterbox-crush.

const { useState, useEffect, useRef, useMemo, useCallback } = React;

// ─── geometry helpers ──────────────────────────────────────────────────────

function nodeById(id) { return window.QOL_DIAGRAM.NODES.find((n) => n.id === id); }

// Presentation policy now lives in data.js under META (tier1, minimalFlow,
// kindStyles). Resolve it lazily inside the layout helpers so editing
// data.js + refreshing the page is enough - no build, no jsx edit.
function readMeta() {
  return (window.QOL_DIAGRAM && window.QOL_DIAGRAM.META) || {};
}

// Pick connection sides automatically: vertical when Y-centers differ
// substantially, horizontal when they're roughly on the same row.
function autoSides(a, b) {
  const aCy = a.y + a.h / 2;
  const bCy = b.y + b.h / 2;
  if (Math.abs(aCy - bCy) > Math.max(a.h, b.h) / 2) {
    return bCy > aCy ? ["bottom", "top"] : ["top", "bottom"];
  }
  return b.x > a.x ? ["right", "left"] : ["left", "right"];
}

// Group nodes into rows by Y-center proximity (within max(h)/2).
function groupRows(nodes) {
  const sorted = [...nodes].sort((a, b) => a.y - b.y);
  const rows = [];
  for (const n of sorted) {
    const last = rows[rows.length - 1];
    if (!last) { rows.push([n]); continue; }
    const lastC = last[0].y + last[0].h / 2;
    const nC    = n.y + n.h / 2;
    const tol   = Math.max(last[0].h, n.h) / 2;
    if (Math.abs(lastC - nC) < tol) last.push(n);
    else rows.push([n]);
  }
  return rows;
}

// Shared compact layout pass: groups nodes per region into rows by
// Y-proximity, drops original vertical gaps, optionally centers each row.
// Both the minimal and descriptive levels go through this; only the input
// filtering, height table, and gap sizing differ.
function computeCompactLayout(srcRegions, srcNodes, opts) {
  const { CANVAS } = window.QOL_DIAGRAM;
  const resize = (n) => ({
    ...n,
    h: opts.heightTable[n.kind] || n.h,
    w: opts.widthTable ? (opts.widthTable[n.kind] || n.w) : n.w,
  });
  const sized = opts.nodeFilter
    ? srcNodes.filter(opts.nodeFilter).map(resize)
    : srcNodes.map(resize);

  const byRegion = new Map();
  for (const n of sized) {
    if (!byRegion.has(n.region)) byRegion.set(n.region, []);
    byRegion.get(n.region).push(n);
  }

  const LABEL_PAD_TOP  = 36;
  const PAD_BOTTOM     = 20;
  const TOP_PAD        = 70;
  const EMPTY_REGION_H = LABEL_PAD_TOP + PAD_BOTTOM;

  let cursor = TOP_PAD;
  const regions = [];
  const nodes = [];

  for (const r of srcRegions) {
    const here = byRegion.get(r.id) || [];
    if (here.length === 0) {
      regions.push({ ...r, y: cursor, h: EMPTY_REGION_H });
      cursor += EMPTY_REGION_H + opts.regionGutter;
      continue;
    }

    const rows = groupRows(here);
    let rowY = cursor + LABEL_PAD_TOP;
    let lastBottom = rowY;
    for (const row of rows) {
      const rowH = Math.max(...row.map((n) => n.h));
      if (opts.centerRows) {
        // Repack horizontally with a fixed gap and center the row. Source
        // gaps grow disproportionately once widths shrink, so cards drift
        // apart without this repack.
        row.sort((a, b) => a.x - b.x);
        const HG = 24;
        const totalW = row.reduce((s, n) => s + n.w, 0) + HG * (row.length - 1);
        let cx = (CANVAS.w - totalW) / 2;
        for (const n of row) {
          nodes.push({ ...n, x: cx, y: rowY });
          cx += n.w + HG;
        }
      } else {
        for (const n of row) nodes.push({ ...n, y: rowY });
      }
      lastBottom = rowY + rowH;
      rowY       = lastBottom + opts.rowGap;
    }

    const regionH = (lastBottom + PAD_BOTTOM) - cursor;
    regions.push({ ...r, y: cursor, h: regionH });
    cursor += regionH + opts.regionGutter;
  }

  return { regions, nodes, canvasH: cursor };
}

function buildSizeTable(kindStyles, hKey, wKey) {
  const h = {}, w = {};
  for (const [kind, s] of Object.entries(kindStyles || {})) {
    if (hKey && s[hKey] != null) h[kind] = s[hKey];
    if (wKey && s[wKey] != null) w[kind] = s[wKey];
  }
  return { heightTable: h, widthTable: wKey ? w : undefined };
}

function computeMinimalLayout(srcRegions, srcNodes) {
  const meta = readMeta();
  const tier1 = new Set(meta.tier1 || []);
  const { heightTable, widthTable } = buildSizeTable(meta.kindStyles, "minimalH", "minimalW");
  return computeCompactLayout(srcRegions, srcNodes, {
    nodeFilter:   (n) => tier1.has(n.id),
    heightTable,
    widthTable,
    centerRows:   true,
    rowGap:       22,
    regionGutter: 36,
  });
}

function computeDescriptiveLayout(srcRegions, srcNodes) {
  const meta = readMeta();
  const { heightTable } = buildSizeTable(meta.kindStyles, "descriptiveH", null);
  return computeCompactLayout(srcRegions, srcNodes, {
    // Drop nodes flagged minimalOnly (e.g. the synthetic p-os
    // platform-layer anchor that exists only so the minimal-view spine
    // arrow has a generic "platform" target). In descriptive + detailed
    // the three real OS cards take over.
    nodeFilter:   (n) => !n.minimalOnly,
    heightTable,
    centerRows:   false,
    rowGap:       14,
    regionGutter: 32,
  });
}

// 5px stand-off so the path geometry stops outside the card border. The
// arrowhead marker (markerWidth 7, refX 9) then sits in that gap with its
// tip just touching the card edge — line stroke never lands on top of the
// 1px card border, eliminating the "scratch over the rectangle" overlap.
const SIDE_GAP = 5;
function sidePoint(node, side) {
  switch (side) {
    case "left":   return [node.x - SIDE_GAP,              node.y + node.h / 2];
    case "right":  return [node.x + node.w + SIDE_GAP,     node.y + node.h / 2];
    case "top":    return [node.x + node.w / 2,            node.y - SIDE_GAP];
    case "bottom": return [node.x + node.w / 2,            node.y + node.h + SIDE_GAP];
  }
}

function orthogonalPath([x1, y1], [x2, y2], fromSide, toSide) {
  // Three-segment Manhattan route with rounded corners. Used by the
  // minimal-mode synthesized edges where clean architectural lines read
  // better than bezier curves.
  if (fromSide === "bottom" && toSide === "top") {
    if (Math.abs(x1 - x2) < 3) return `M ${x1} ${y1} L ${x2} ${y2}`;
    const midY = (y1 + y2) / 2;
    const sgn  = x2 > x1 ? 1 : -1;
    const cr   = Math.min(12, Math.abs(x2 - x1) / 2, Math.abs(midY - y1), Math.abs(y2 - midY));
    return `M ${x1} ${y1}
            L ${x1} ${midY - cr}
            Q ${x1} ${midY}, ${x1 + cr * sgn} ${midY}
            L ${x2 - cr * sgn} ${midY}
            Q ${x2} ${midY}, ${x2} ${midY + cr}
            L ${x2} ${y2}`;
  }
  return `M ${x1} ${y1} L ${x2} ${y2}`;
}

function bezierPath(a, b, edge) {
  if (edge.route === "orthogonal") return orthogonalPath(a, b, edge.fromSide, edge.toSide);
  const [x1, y1] = a;
  const [x2, y2] = b;
  const horiz = edge.fromSide === "left" || edge.fromSide === "right" ||
                edge.toSide   === "left" || edge.toSide   === "right";
  if (edge.bypass) {
    // Right-side bypass: drop down outside the bands then arc back in.
    const railX = Math.max(x1, x2) + 80;
    return `M ${x1} ${y1} C ${railX} ${y1}, ${railX} ${y2}, ${x2} ${y2}`;
  }
  if (edge.longRail) {
    // Long off-stage rail edge: source side → side rail → target. Right-angle
    // L path with small chamfers so the line reads as "out of band" plumbing.
    const railX = edge.longRail === "left" ? 30 : (window.QOL_DIAGRAM.CANVAS.w - 30);
    const k = 14; // chamfer
    const goDown = y2 > y1;
    const railEntryY1 = y1 + (goDown ? k : -k);
    const railEntryY2 = y2 + (goDown ? -k : k);
    return `M ${x1} ${y1}
            L ${railX + (railX > x1 ? -k : k)} ${y1}
            Q ${railX} ${y1}, ${railX} ${railEntryY1}
            L ${railX} ${railEntryY2}
            Q ${railX} ${y2}, ${railX + (railX > x2 ? -k : k)} ${y2}
            L ${x2} ${y2}`;
  }
  if (edge.wrap) {
    // Wrap-around: from bottom of right-side node, down then left then up into top of left-side node.
    const railY = (y1 + y2) / 2 + 24;
    return `M ${x1} ${y1} L ${x1} ${railY} L ${x2} ${railY} L ${x2} ${y2}`;
  }
  if (edge.dropX !== undefined) {
    // Vertical drop to a specific X then over.
    return `M ${x1} ${y1} C ${x1} ${y2}, ${x2} ${y1}, ${x2} ${y2}`;
  }
  if (edge.sideLoopReturn) {
    // Bottom-to-bottom loop arcing below both nodes.
    const arcY = Math.max(y1, y2) + 28;
    return `M ${x1} ${y1} C ${x1} ${arcY}, ${x2} ${arcY}, ${x2} ${y2}`;
  }
  if (horiz) {
    const dx = Math.max(40, Math.abs(x2 - x1) * 0.35);
    const c1x = x1 + (edge.fromSide === "right" ?  dx : -dx);
    const c2x = x2 + (edge.toSide   === "right" ?  dx : -dx);
    return `M ${x1} ${y1} C ${c1x} ${y1}, ${c2x} ${y2}, ${x2} ${y2}`;
  }
  const dy = Math.max(20, Math.abs(y2 - y1) * 0.45);
  const c1y = y1 + (edge.fromSide === "bottom" ?  dy : -dy);
  const c2y = y2 + (edge.toSide   === "bottom" ?  dy : -dy);
  return `M ${x1} ${y1} C ${x1} ${c1y}, ${x2} ${c2y}, ${x2} ${y2}`;
}

// ─── primitive renderers ───────────────────────────────────────────────────

function Region({ region }) {
  const accentVar = region.accent ? `var(--${region.accent})` : "var(--ink-3)";
  return (
    <div className="region"
         data-region-id={region.id}
         data-region-ord={region.ord}
         data-boundary={region.boundary || ""}
         data-entry={region.entry ? "true" : undefined}
         style={{
           left: region.x, top: region.y, width: region.w, height: region.h,
           "--region-accent": accentVar,
         }}>
      <div className="region-label">
        <span className="region-ord">{region.ord}</span>
        <span className="region-title">{region.title}</span>
        <span className="region-caption">{region.caption}</span>
        {region.lifetime && <span className="region-lifetime">[{region.lifetime}]</span>}
      </div>
    </div>
  );
}

function Node({ node, accent, highlighted, dimmed, traced, expanded, onHover, onLeave, onClick }) {
  const cls = [
    "node", `kind-${node.kind}`,
    highlighted ? "is-hover" : "",
    dimmed ? "is-dim" : "",
    traced ? "is-traced" : "",
    expanded ? "is-expanded" : "",
    node.platform ? `platform-${node.platform}` : "",
  ].join(" ");
  const accentVar = accent ? `var(--${accent})` : "var(--ink-3)";
  return (
    <div className={cls}
         data-node-id={node.id}
         data-region={node.region}
         style={{
           left: node.x, top: node.y, width: node.w, height: node.h,
           "--region-accent": accentVar,
         }}
         onMouseEnter={() => onHover(node.id)}
         onMouseLeave={onLeave}
         onClick={() => onClick && onClick(node.id)}>
      <NodeBody node={node} />
    </div>
  );
}

function NodeBody({ node }) {
  if (node.kind === "platform") {
    return (
      <div className="platform-card">
        <div className="platform-header">
          <span className="platform-glyph">{node.platform === "linux" ? "L" : node.platform === "macos" ? "M" : "W"}</span>
          <span className="platform-name">{node.label}</span>
        </div>
        <ul className="platform-bullets">
          {node.bullets.map((b, i) => <li key={i}>{b}</li>)}
        </ul>
        {node.note && <div className="platform-note">{node.note}</div>}
        {node.code && <div className="node-code">{node.code}</div>}
      </div>
    );
  }
  if (node.kind === "ext") {
    return (
      <div className="ext-card">
        <div className="ext-row">
          <span className="ext-label">{node.label}</span>
          <span className="ext-pid">{node.pid}</span>
        </div>
        <div className="ext-sub">{node.sub}</div>
        {node.internals && (
          <ul className="ext-internals">
            {node.internals.map((it, i) => <li key={i}>{it}</li>)}
          </ul>
        )}
        <div className="ext-meta">
          <span>plugin.toml</span>
          <span className={`ext-daemon ${node.daemon ? "on" : "off"}`}>
            {node.daemon ? "▣ daemon" : "□ ephemeral"}
          </span>
        </div>
      </div>
    );
  }
  if (node.kind === "api") {
    return (
      <div className="api-card">
        <div className="api-row">
          <span className="api-label">{node.label}</span>
          <span className="api-port">127.0.0.1:42700</span>
        </div>
        <div className="api-sub">{node.sub}</div>
        {node.ipc && <div className="api-ipc">{node.ipc}</div>}
        {node.code && <div className="node-code">{node.code}</div>}
      </div>
    );
  }
  if (node.kind === "state") {
    return (
      <div className="state-card">
        <div className="api-row">
          <span className="api-label">{node.label}</span>
          <span className="state-badge">unix · read-only</span>
        </div>
        <div className="api-sub">{node.sub}</div>
        {node.ipc && <div className="api-ipc">{node.ipc}</div>}
        {node.code && <div className="node-code">{node.code}</div>}
      </div>
    );
  }
  if (node.kind === "router") {
    return (
      <div className="router-card">
        <div className="api-row">
          <span className="api-label">{node.label}</span>
          <span className="router-badge">OS thread</span>
        </div>
        <div className="api-sub">{node.sub}</div>
        {node.ipc && <div className="api-ipc">{node.ipc}</div>}
        {node.code && <div className="node-code">{node.code}</div>}
      </div>
    );
  }
  if (node.kind === "ps-file") {
    return (
      <div className="psfile-card">
        <div className="psfile-head">
          <span className="psfile-label">{node.label}</span>
          {node.env && <span className="psfile-env">env: {node.env}</span>}
        </div>
        <div className="psfile-path">{node.path}</div>
        <div className="psfile-rw">
          {node.writes && node.writes.length > 0 && (
            <span className="psfile-w" title="written by">
              <span className="rw-glyph">w</span>
              {node.writes.join(" · ")}
            </span>
          )}
          {node.reads && node.reads.length > 0 && (
            <span className="psfile-r" title="read by">
              <span className="rw-glyph">r</span>
              {node.reads.join(" · ")}
            </span>
          )}
        </div>
      </div>
    );
  }
  if (node.kind === "store") {
    return (
      <div className={`store-card ${node.ephemeral ? "is-ephemeral" : ""}`}>
        <div className="store-row">
          <span className="generic-label">{node.label}</span>
          {node.ephemeral && <span className="store-badge">ephemeral</span>}
        </div>
        {node.sub && <div className="generic-sub">{node.sub}</div>}
        {node.path && <div className="store-path">{node.path}</div>}
      </div>
    );
  }
  // core / generic — supports an optional bullets list (e.g. event bus variants),
  // a `note` field (small italic footer), and the `plug-anchor` variant for fan-in nodes.
  return (
    <div className="generic">
      <div className="generic-label">{node.label}</div>
      {node.sub && <div className="generic-sub">{node.sub}</div>}
      {node.bullets && (
        <ul className="generic-bullets">
          {node.bullets.map((b, i) => <li key={i}>{b}</li>)}
        </ul>
      )}
      {node.note && <div className="generic-note">{node.note}</div>}
      {node.code && <div className="node-code">{node.code}</div>}
    </div>
  );
}

// ─── band gutter chevrons ──────────────────────────────────────────────────

function Gutters() {
  const { GUTTERS, CANVAS } = window.QOL_DIAGRAM;
  const cx = CANVAS.w / 2;
  return (
    <svg className="gutters" viewBox={`0 0 ${CANVAS.w} ${CANVAS.h}`} preserveAspectRatio="none">
      {GUTTERS.map((g, i) => {
        const mid = (g.fromY + g.toY) / 2;
        const top = g.fromY + 4;
        const bot = g.toY   - 4;
        const cls = `gutter ${g.dashed ? "is-dashed" : ""} tone-${g.tone || "ink"}`;
        return (
          <g key={i} className={cls}>
            <line x1={cx} y1={top}     x2={cx} y2={bot - 8} className="gutter-line" />
            {/* chevron */}
            <path d={`M ${cx - 6} ${bot - 10} L ${cx} ${bot} L ${cx + 6} ${bot - 10}`} className="gutter-chev" />
            {/* label */}
            <text x={cx + 14} y={mid + 3} className="gutter-label">{g.label}</text>
          </g>
        );
      })}
    </svg>
  );
}

// ─── edges layer ───────────────────────────────────────────────────────────

function Edges({ nodes, edges, tracedPairs, canvasH }) {
  const { CANVAS } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  const nodeMap = useMemo(() => new Map(nodes.map((n) => [n.id, n])), [nodes]);
  const staticPaths = useMemo(() => {
    return edges.map((edge, i) => {
      const from = nodeMap.get(edge.from);
      const to   = nodeMap.get(edge.to);
      if (!from || !to) return null;
      const [autoFrom, autoTo] = autoSides(from, to);
      const fromSide = edge.fromSide || autoFrom;
      const toSide   = edge.toSide   || autoTo;
      const a = sidePoint(from, fromSide);
      const b = sidePoint(to,   toSide);
      return { i, edge: { ...edge, fromSide, toSide }, d: bezierPath(a, b, { ...edge, fromSide, toSide }) };
    }).filter(Boolean);
  }, [edges, nodeMap]);

  // Compute trace overlay paths (built from active trace; not from EDGES).
  const tracePaths = useMemo(() => {
    if (!tracedPairs) return [];
    return tracedPairs.paths;
  }, [tracedPairs]);

  return (
    <svg className="edges" viewBox={`0 0 ${CANVAS.w} ${h}`} preserveAspectRatio="none">
      <defs>
        <marker id="arrow-ink"   viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M 0 1 L 9 5 L 0 9 z" className="arrow-ink" />
        </marker>
        <marker id="arrow-amber" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M 0 1 L 9 5 L 0 9 z" className="arrow-amber" />
        </marker>
        <marker id="arrow-slate" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse">
          <path d="M 0 1 L 9 5 L 0 9 z" className="arrow-slate" />
        </marker>
      </defs>

      {staticPaths.map(({ i, edge, d }) => {
        const cls = [
          "edge", `tone-${edge.tone || "ink"}`,
          edge.dashed ? "is-dashed" : "",
          edge.hairline ? "is-hairline" : "",
          edge.internal ? "is-internal" : "",
          edge.bypass ? "is-bypass" : "",
          edge.longRail ? "is-longrail" : "",
          tracedPairs ? "is-fade" : "",
        ].join(" ");
        // Every edge gets an arrowhead — flow direction has to be readable
        // without context (Tufte: lines are multivocal; explicit terminators
        // pin down the semantic). Originally only internal/bypass/longRail
        // edges showed arrows; cross-region edges (spine) were left without,
        // which made minimal view read as a passive web rather than a flow.
        const showArrow = true;
        return (
          <path key={`s-${i}`} d={d} className={cls}
                markerEnd={showArrow ? `url(#arrow-${edge.tone || "ink"})` : ""} />
        );
      })}

      {tracePaths.map((d, i) => (
        <path key={`t-${i}`} d={d} className={`edge trace-edge tone-${tracedPairs.tone}`}
              markerEnd={`url(#arrow-${tracedPairs.tone})`} />
      ))}
    </svg>
  );
}

// ─── main diagram ──────────────────────────────────────────────────────────

function Diagram({ tweaks, setTweak }) {
  const { REGIONS: SRC_REGIONS, NODES: SRC_NODES, TRACES, CANVAS } = window.QOL_DIAGRAM;
  const level   = tweaks.level || "minimal";
  const compact = level === "minimal";

  const [hoverId, setHoverId] = useState(null);
  const [expandedIds, setExpandedIds] = useState(() => new Set());
  const [activeTrace, setActiveTrace] = useState(null);
  const [scale, setScale] = useState(1);
  const outerRef = useRef(null);

  const toggleExpand = useCallback((id) => {
    setExpandedIds((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }, []);

  // minimal      - tier-1 nodes only, row-packed, centered.
  // descriptive  - all nodes, row-packed in original columns, shorter cards.
  // detailed     - the source layout from data.js, all cards fully rendered.
  const layout = useMemo(() => {
    if (level === "minimal")     return computeMinimalLayout(SRC_REGIONS, SRC_NODES);
    if (level === "descriptive") return computeDescriptiveLayout(SRC_REGIONS, SRC_NODES);
    // Detailed: source layout, but strip minimalOnly synthetics.
    return { regions: SRC_REGIONS, nodes: SRC_NODES.filter((n) => !n.minimalOnly), canvasH: CANVAS.h };
  }, [level, SRC_REGIONS, SRC_NODES, CANVAS.h]);

  const REGIONS  = layout.regions;
  const NODES    = layout.nodes;
  const CANVAS_H = layout.canvasH;

  // Layout-aware node lookup. Both trace overlays and the edges layer need
  // positions from the current layout pass, not the source coords.
  const nodeMap = useMemo(() => new Map(NODES.map((n) => [n.id, n])), [NODES]);

  // Minimal renders synthesized inter-region flow (META.minimalFlow);
  // descriptive and detailed render the full edge set from data.js.
  const SRC_EDGES = window.QOL_DIAGRAM.EDGES;
  const META = window.QOL_DIAGRAM.META || {};
  const visibleEdges = useMemo(() => {
    return level === "minimal" ? (META.minimalFlow || []) : SRC_EDGES;
  }, [level, SRC_EDGES, META.minimalFlow]);

  // Map region id → accent token name so Region/Node/FlowChevrons can write
  // the --region-accent CSS custom property inline. Means the stylesheet
  // never has to know which region uses which accent.
  const accentByRegion = useMemo(() => {
    const m = new Map();
    for (const r of REGIONS) m.set(r.id, r.accent || "ink-3");
    return m;
  }, [REGIONS]);

  // Switching back to minimal drops the compact-incompatible trace overlay,
  // since the compact layout omits any non-tier-1 trace step.
  useEffect(() => {
    if (compact && activeTrace) setActiveTrace(null);
  }, [compact, activeTrace]);

  // Card positions and sizes change between layouts; collapse any inline
  // expansions when the level switches so an expanded card from descriptive
  // does not awkwardly persist into minimal where its absolute slot is gone.
  useEffect(() => {
    setExpandedIds(new Set());
  }, [level]);

  // Fit-to-width scaling (capped at 1×). Stage scrolls vertically.
  useEffect(() => {
    function fit() {
      const topbar = document.querySelector(".topbar");
      const topH = topbar ? topbar.getBoundingClientRect().height : 56;
      const pad = 32;
      const vw  = window.innerWidth - pad * 2;
      const vh  = window.innerHeight - topH - pad * 2;
      let s = vw / CANVAS.w;
      s = Math.min(s, 1.0);
      const fitsHeight = CANVAS_H * s <= vh;
      if (fitsHeight) {
        s = Math.min(vh / CANVAS_H, 1.0, vw / CANVAS.w);
      }
      setScale(Math.max(0.35, s));
    }
    fit();
    window.addEventListener("resize", fit);
    const t = setTimeout(fit, 300);
    return () => { window.removeEventListener("resize", fit); clearTimeout(t); };
  }, [CANVAS.w, CANVAS_H]);

  // Trace state derivations.
  const tracedNodeIds = useMemo(() => activeTrace ? new Set(activeTrace.steps) : null, [activeTrace]);
  const tracedPairs   = useMemo(() => {
    if (!activeTrace) return null;
    const paths = [];
    for (let i = 0; i < activeTrace.steps.length - 1; i++) {
      const a = nodeMap.get(activeTrace.steps[i]);
      const b = nodeMap.get(activeTrace.steps[i + 1]);
      if (!a || !b) continue;
      const [fromSide, toSide] = autoSides(a, b);
      const pa = sidePoint(a, fromSide);
      const pb = sidePoint(b, toSide);
      paths.push(bezierPath(pa, pb, { fromSide, toSide }));
    }
    return { paths, tone: activeTrace.tone || "ink" };
  }, [activeTrace, nodeMap]);

  const onSelectTrace = (t) => {
    // Trace nodes belong to the original layout, not the compact one,
    // so picking a trace while in minimal also expands the diagram.
    if (compact && setTweak) setTweak("level", "descriptive");
    setActiveTrace((prev) => (prev && prev.id === t.id ? null : t));
    setExpandedIds(new Set());
  };

  return (
    <div className="diagram-root">
      <Topbar
        activeTrace={activeTrace} traces={TRACES} onSelectTrace={onSelectTrace}
        level={tweaks.level || "minimal"}
        setLevel={(l) => setTweak && setTweak("level", l)}
      />

      <div className="stage-outer" ref={outerRef}>
        <div className="stage-wrap" style={{ width: CANVAS.w * scale, height: CANVAS_H * scale }}>
          <div className="stage" style={{ width: CANVAS.w, height: CANVAS_H, transform: `scale(${scale})` }}>
            <PaperBackdrop />

            {REGIONS.map((r) => <Region key={r.id} region={r} />)}

            {level !== "detailed" && <FlowChevrons regions={REGIONS} canvasW={CANVAS.w} canvasH={CANVAS_H} accentByRegion={accentByRegion} />}
            {level === "detailed" && <Quadrants />}
            {level === "detailed" && <Gutters />}
            {level === "detailed" && <TokioBoundary />}
            {level === "detailed" && <LaneLabels />}
            <Edges nodes={NODES} edges={visibleEdges} tracedPairs={tracedPairs} canvasH={CANVAS_H} />

            {NODES.map((n) => {
              const dimmed = tracedNodeIds && !tracedNodeIds.has(n.id);
              const traced = tracedNodeIds && tracedNodeIds.has(n.id);
              const highlighted = hoverId === n.id;
              const expanded = expandedIds.has(n.id);
              return (
                <Node key={n.id} node={n} accent={accentByRegion.get(n.region)}
                      highlighted={highlighted} dimmed={dimmed} traced={traced}
                      expanded={expanded}
                      onHover={setHoverId} onLeave={() => setHoverId(null)}
                      onClick={toggleExpand} />
              );
            })}

            <CornerMarks canvasH={CANVAS_H} />
            <PlateAnnotations activeTrace={activeTrace} canvasH={CANVAS_H} />
          </div>
        </div>
      </div>

      <DetailPanel activeTrace={activeTrace}
                   onClose={() => setActiveTrace(null)} />
    </div>
  );
}

// Downward chevrons between consecutive regions - reads as a numbered flow
// path through r1 → r2 → ... → r6. Computed from the current layout so it
// follows minimal and descriptive compaction, not the source coords.
function FlowChevrons({ regions, canvasW, canvasH, accentByRegion }) {
  if (regions.length < 2) return null;
  const cx = canvasW / 2;
  return (
    <svg className="flow-chevrons" viewBox={`0 0 ${canvasW} ${canvasH}`} preserveAspectRatio="none">
      {regions.slice(0, -1).map((r, i) => {
        const next = regions[i + 1];
        const cy = (r.y + r.h + next.y) / 2;
        const accent = accentByRegion && accentByRegion.get(r.id);
        const style = accent ? { "--region-accent": `var(--${accent})` } : undefined;
        return (
          <g key={i} className="flow-chev" style={style}>
            <path d={`M ${cx - 18} ${cy - 8} L ${cx} ${cy + 6} L ${cx + 18} ${cy - 8}`} />
          </g>
        );
      })}
    </svg>
  );
}

function Quadrants() {
  const { META, CANVAS } = window.QOL_DIAGRAM;
  if (!META || !META.quadrants) return null;
  // Compute the cross-divider extents from the four quadrant bounds.
  const xs = META.quadrants.map((q) => q.x);
  const ws = META.quadrants.map((q) => q.x + q.w);
  const ys = META.quadrants.map((q) => q.y);
  const hs = META.quadrants.map((q) => q.y + q.h);
  const left = Math.min(...xs), right = Math.max(...ws);
  const top  = Math.min(...ys), bot   = Math.max(...hs);
  // Vertical divider between left & right halves.
  const midX = (Math.max(...xs.filter((x, i) => META.quadrants[i].id.endsWith("l"))) +
                Math.min(...xs.filter((x, i) => META.quadrants[i].id.endsWith("r")))) / 2;
  // Horizontal divider between top & bottom halves.
  const midY = (Math.max(...ys.filter((y, i) => META.quadrants[i].id.startsWith("t"))) +
                Math.min(...ys.filter((y, i) => META.quadrants[i].id.startsWith("b")))) / 2;

  return (
    <React.Fragment>
      {META.quadrants.map((q) => (
        <div key={q.id} className={`quadrant quadrant-${q.id}`}
             style={{ left: q.x, top: q.y, width: q.w, height: q.h }}>
          <div className="quadrant-label">
            <span className="quadrant-glyph">{q.glyph}</span>
            <span className="quadrant-ord">{q.ord}</span>
            <span className="quadrant-title">{q.title}</span>
            <span className="quadrant-axes">
              <span>{q.axisX}</span>
              <span className="axes-dot">·</span>
              <span>{q.axisY}</span>
            </span>
          </div>
        </div>
      ))}
      <svg className="quadrant-cross" viewBox={`0 0 ${CANVAS.w} ${CANVAS.h}`} preserveAspectRatio="none">
        <line x1={midX} y1={top}  x2={midX} y2={bot} className="qcross-line" />
        <line x1={left} y1={midY} x2={right} y2={midY} className="qcross-line" />
      </svg>
    </React.Fragment>
  );
}

function TokioBoundary() {
  const { META, REGIONS } = window.QOL_DIAGRAM;
  if (!META || !META.tokioBoundary) return null;
  const r3 = REGIONS.find((r) => r.id === "r3");
  if (!r3) return null;
  const y = META.tokioBoundary.y;
  const left = r3.x + 12;
  const right = r3.x + r3.w - 12;
  return (
    <svg className="tokio-boundary" viewBox={`0 0 ${window.QOL_DIAGRAM.CANVAS.w} ${window.QOL_DIAGRAM.CANVAS.h}`}
         preserveAspectRatio="none">
      {/* The dashed line itself */}
      <line x1={left} y1={y} x2={right} y2={y} className="tokio-rule" />
      {/* End ticks */}
      <line x1={left}  y1={y - 6} x2={left}  y2={y + 6} className="tokio-tick" />
      <line x1={right} y1={y - 6} x2={right} y2={y + 6} className="tokio-tick" />
      {/* Centered label */}
      <rect x={(left + right) / 2 - 110} y={y - 11} width="220" height="22" className="tokio-label-bg" />
      <text x={(left + right) / 2} y={y + 4} className="tokio-label-text">
        ↓  tokio multi-thread runtime  ↓
      </text>
      {/* Phase labels at the ends */}
      <text x={left + 6} y={y - 12} className="tokio-phase-text">pre-tokio · main thread</text>
      <text x={right - 6} y={y - 12} className="tokio-phase-text" textAnchor="end">async tasks</text>
    </svg>
  );
}

function LaneLabels() {
  const { META, CANVAS } = window.QOL_DIAGRAM;
  if (!META || !META.laneLabels) return null;
  return (
    <svg className="lane-labels" viewBox={`0 0 ${CANVAS.w} ${CANVAS.h}`} preserveAspectRatio="none">
      {META.laneLabels.map((l, i) => (
        <g key={i} transform={`translate(${l.x}, ${l.y + l.h / 2})`}>
          <text className="lane-label-text" transform="rotate(-90)">{l.label}</text>
        </g>
      ))}
    </svg>
  );
}

function PaperBackdrop() { return <div className="paper-backdrop" aria-hidden="true" />; }

function CornerMarks({ canvasH }) {
  const { CANVAS } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  const mark = (x, y, dx, dy) => (
    <g>
      <line x1={x} y1={y} x2={x + dx} y2={y} className="mark" />
      <line x1={x} y1={y} x2={x} y2={y + dy} className="mark" />
    </g>
  );
  return (
    <svg className="corner-marks" viewBox={`0 0 ${CANVAS.w} ${h}`}>
      {mark(8, 8,  24,  24)}
      {mark(CANVAS.w - 8, 8,  -24,  24)}
      {mark(8, h - 8,  24, -24)}
      {mark(CANVAS.w - 8, h - 8, -24, -24)}
    </svg>
  );
}

function PlateAnnotations({ activeTrace, canvasH }) {
  const { CANVAS, META } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  return (
    <div className="plate-annotations" style={{ width: CANVAS.w, height: h }}>
      <div className="plate plate-tl">
        <div className="plate-line">fig · 01</div>
        <div className="plate-line plate-mono">runtime architecture map</div>
      </div>
      <div className="plate plate-tr">
        <div className="plate-line">qol-tray</div>
        <div className="plate-line plate-mono">v3.15.1 · main</div>
      </div>
      <div className="plate plate-bl plate-mono">
        {META && META.binaries}
      </div>
      <div className="plate plate-br plate-mono">
        {activeTrace ? `trace · ${activeTrace.ord} · ${activeTrace.label}` : "trace · idle"}
      </div>
    </div>
  );
}

// ─── topbar ────────────────────────────────────────────────────────────────

function Topbar({ activeTrace, traces, onSelectTrace, level, setLevel }) {
  const [traceOpen, setTraceOpen] = useState(false);
  const menuRef = useRef(null);

  useEffect(() => {
    if (!traceOpen) return;
    function onDocClick(e) {
      if (menuRef.current && !menuRef.current.contains(e.target)) setTraceOpen(false);
    }
    document.addEventListener("mousedown", onDocClick);
    return () => document.removeEventListener("mousedown", onDocClick);
  }, [traceOpen]);

  return (
    <div className="topbar">
      <div className="topbar-left">
        <span className="topbar-title">QoL Tray</span>
        <span className="topbar-sep">·</span>
        <span className="topbar-sub">Runtime Architecture Map</span>
      </div>

      <div className="topbar-center" ref={menuRef}>
        <button className={`trace-toggle ${activeTrace ? "is-active" : ""}`}
                onClick={() => setTraceOpen((o) => !o)}>
          <span className="trace-toggle-ico">▷</span>
          <span className="trace-toggle-label">
            {activeTrace ? <>trace · <b>{activeTrace.ord}</b> {activeTrace.label}</> : "traces"}
          </span>
          {activeTrace ? (
            <span className="trace-toggle-stop"
                  onClick={(e) => { e.stopPropagation(); onSelectTrace(activeTrace); }}>×</span>
          ) : (
            <span className="trace-toggle-caret">{traceOpen ? "▴" : "▾"}</span>
          )}
        </button>

        {traceOpen && (
          <div className="trace-menu">
            <div className="trace-menu-eyebrow">runtime trace</div>
            {traces.map((t) => (
              <button key={t.id}
                      className={`trace-menu-item ${activeTrace && activeTrace.id === t.id ? "is-on" : ""} ${t.tone === "amber" ? "tone-amber" : ""}`}
                      onClick={() => { onSelectTrace(t); setTraceOpen(false); }}>
                <span className="trace-ord">{t.ord}</span>
                <span className="trace-label">{t.label}</span>
                <span className="trace-play">{activeTrace && activeTrace.id === t.id ? "■" : "▷"}</span>
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="topbar-right">
        <span className="topbar-eyebrow">level</span>
        {["minimal","descriptive","detailed"].map((l) => (
          <button key={l} className={`pill ${level === l ? "is-on" : ""}`}
                  onClick={() => setLevel(l)}>{l}</button>
        ))}
      </div>
    </div>
  );
}

// ─── detail panel ──────────────────────────────────────────────────────────

// Trace narrative side panel. Node details are now shown inline by clicking
// the card to toggle expansion, so this panel is trace-only.
function DetailPanel({ activeTrace, onClose }) {
  if (!activeTrace) return null;
  return (
    <div className="detail-panel">
      <button className="detail-close" onClick={onClose} aria-label="close">×</button>
      <div className="detail-section">
        <div className="detail-eyebrow">trace · {activeTrace.ord}</div>
        <div className="detail-title">{activeTrace.label}</div>
        <div className="detail-body">{activeTrace.narrative}</div>
        <div className="detail-trace-steps">
          {activeTrace.steps.map((id, i) => {
            const n = nodeById(id);
            return (
              <React.Fragment key={i}>
                <span className="step">{n ? n.label : id}</span>
                {i < activeTrace.steps.length - 1 && <span className="step-sep">↓</span>}
              </React.Fragment>
            );
          })}
        </div>
      </div>
    </div>
  );
}

window.Diagram = Diagram;

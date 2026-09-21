// AUTO-GENERATED from the matching .jsx file via build.mjs. Edit the .jsx, then run `npm run build`.
(function () {
"use strict";
// Diagram for the QoL Tray Runtime Architecture Map (top-down).
// Vertical bands stacked on a 1320 × 1960 canvas. Stage scales to fit the
// viewport WIDTH (capped at 1×) and scrolls vertically — no letterbox-crush.

const {
  useState,
  useEffect,
  useRef,
  useMemo,
  useCallback
} = React;

// ─── geometry helpers ──────────────────────────────────────────────────────

function nodeById(id) {
  return window.QOL_DIAGRAM.NODES.find(n => n.id === id);
}

// Presentation policy now lives in data.js under META (tier1, minimalFlow,
// kindStyles). Resolve it lazily inside the layout helpers so editing
// data.js + refreshing the page is enough - no build, no jsx edit.
function readMeta() {
  return window.QOL_DIAGRAM && window.QOL_DIAGRAM.META || {};
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
    if (!last) {
      rows.push([n]);
      continue;
    }
    const lastC = last[0].y + last[0].h / 2;
    const nC = n.y + n.h / 2;
    const tol = Math.max(last[0].h, n.h) / 2;
    if (Math.abs(lastC - nC) < tol) last.push(n);else rows.push([n]);
  }
  return rows;
}

// Shared compact layout pass: groups nodes per region into rows by
// Y-proximity, drops original vertical gaps, optionally centers each row.
// Both the minimal and descriptive levels go through this; only the input
// filtering, height table, and gap sizing differ.
function computeCompactLayout(srcRegions, srcNodes, opts) {
  const {
    CANVAS
  } = window.QOL_DIAGRAM;
  const resize = n => ({
    ...n,
    h: opts.heightTable[n.kind] || n.h,
    w: opts.widthTable ? opts.widthTable[n.kind] || n.w : n.w
  });
  const sized = opts.nodeFilter ? srcNodes.filter(opts.nodeFilter).map(resize) : srcNodes.map(resize);
  const byRegion = new Map();
  for (const n of sized) {
    if (!byRegion.has(n.region)) byRegion.set(n.region, []);
    byRegion.get(n.region).push(n);
  }
  const LABEL_PAD_TOP = 36;
  const PAD_BOTTOM = 20;
  const TOP_PAD = 70;
  const EMPTY_REGION_H = LABEL_PAD_TOP + PAD_BOTTOM;
  let cursor = TOP_PAD;
  const regions = [];
  const nodes = [];
  for (const r of srcRegions) {
    const here = byRegion.get(r.id) || [];
    if (here.length === 0) {
      regions.push({
        ...r,
        y: cursor,
        h: EMPTY_REGION_H
      });
      cursor += EMPTY_REGION_H + opts.regionGutter;
      continue;
    }
    const rows = groupRows(here);
    let rowY = cursor + LABEL_PAD_TOP;
    let lastBottom = rowY;
    for (const row of rows) {
      const rowH = Math.max(...row.map(n => n.h));
      if (opts.centerRows) {
        // Repack horizontally with a fixed gap and center the row. Source
        // gaps grow disproportionately once widths shrink, so cards drift
        // apart without this repack.
        row.sort((a, b) => a.x - b.x);
        const HG = 24;
        const totalW = row.reduce((s, n) => s + n.w, 0) + HG * (row.length - 1);
        let cx = (CANVAS.w - totalW) / 2;
        for (const n of row) {
          nodes.push({
            ...n,
            x: cx,
            y: rowY
          });
          cx += n.w + HG;
        }
      } else {
        for (const n of row) nodes.push({
          ...n,
          y: rowY
        });
      }
      lastBottom = rowY + rowH;
      rowY = lastBottom + opts.rowGap;
    }
    const regionH = lastBottom + PAD_BOTTOM - cursor;
    regions.push({
      ...r,
      y: cursor,
      h: regionH
    });
    cursor += regionH + opts.regionGutter;
  }
  return {
    regions,
    nodes,
    canvasH: cursor
  };
}
function buildSizeTable(kindStyles, hKey, wKey) {
  const h = {},
    w = {};
  for (const [kind, s] of Object.entries(kindStyles || {})) {
    if (hKey && s[hKey] != null) h[kind] = s[hKey];
    if (wKey && s[wKey] != null) w[kind] = s[wKey];
  }
  return {
    heightTable: h,
    widthTable: wKey ? w : undefined
  };
}
function computeMinimalLayout(srcRegions, srcNodes) {
  const meta = readMeta();
  const tier1 = new Set(meta.tier1 || []);
  const {
    heightTable,
    widthTable
  } = buildSizeTable(meta.kindStyles, "minimalH", "minimalW");
  return computeCompactLayout(srcRegions, srcNodes, {
    nodeFilter: n => tier1.has(n.id),
    heightTable,
    widthTable,
    centerRows: true,
    rowGap: 22,
    regionGutter: 36
  });
}
function computeDescriptiveLayout(srcRegions, srcNodes) {
  const meta = readMeta();
  const {
    heightTable
  } = buildSizeTable(meta.kindStyles, "descriptiveH", null);
  return computeCompactLayout(srcRegions, srcNodes, {
    // Drop nodes flagged minimalOnly (e.g. the synthetic p-os
    // platform-layer anchor that exists only so the minimal-view spine
    // arrow has a generic "platform" target). In descriptive + detailed
    // the three real OS cards take over.
    nodeFilter: n => !n.minimalOnly,
    heightTable,
    centerRows: false,
    rowGap: 14,
    regionGutter: 32
  });
}

// 5px stand-off so the path geometry stops outside the card border. The
// arrowhead marker (markerWidth 7, refX 9) then sits in that gap with its
// tip just touching the card edge — line stroke never lands on top of the
// 1px card border, eliminating the "scratch over the rectangle" overlap.
const SIDE_GAP = 5;
function sidePoint(node, side) {
  switch (side) {
    case "left":
      return [node.x - SIDE_GAP, node.y + node.h / 2];
    case "right":
      return [node.x + node.w + SIDE_GAP, node.y + node.h / 2];
    case "top":
      return [node.x + node.w / 2, node.y - SIDE_GAP];
    case "bottom":
      return [node.x + node.w / 2, node.y + node.h + SIDE_GAP];
  }
}
function orthogonalPath([x1, y1], [x2, y2], fromSide, toSide) {
  // Three-segment Manhattan route with rounded corners. Used by the
  // minimal-mode synthesized edges where clean architectural lines read
  // better than bezier curves.
  if (fromSide === "bottom" && toSide === "top") {
    if (Math.abs(x1 - x2) < 3) return `M ${x1} ${y1} L ${x2} ${y2}`;
    const midY = (y1 + y2) / 2;
    const sgn = x2 > x1 ? 1 : -1;
    const cr = Math.min(12, Math.abs(x2 - x1) / 2, Math.abs(midY - y1), Math.abs(y2 - midY));
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
  const horiz = edge.fromSide === "left" || edge.fromSide === "right" || edge.toSide === "left" || edge.toSide === "right";
  if (edge.bypass) {
    // Right-side bypass: drop down outside the bands then arc back in.
    const railX = Math.max(x1, x2) + 80;
    return `M ${x1} ${y1} C ${railX} ${y1}, ${railX} ${y2}, ${x2} ${y2}`;
  }
  if (edge.longRail) {
    // Long off-stage rail edge: source side → side rail → target. Right-angle
    // L path with small chamfers so the line reads as "out of band" plumbing.
    const railX = edge.longRail === "left" ? 30 : window.QOL_DIAGRAM.CANVAS.w - 30;
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
    const c1x = x1 + (edge.fromSide === "right" ? dx : -dx);
    const c2x = x2 + (edge.toSide === "right" ? dx : -dx);
    return `M ${x1} ${y1} C ${c1x} ${y1}, ${c2x} ${y2}, ${x2} ${y2}`;
  }
  const dy = Math.max(20, Math.abs(y2 - y1) * 0.45);
  const c1y = y1 + (edge.fromSide === "bottom" ? dy : -dy);
  const c2y = y2 + (edge.toSide === "bottom" ? dy : -dy);
  return `M ${x1} ${y1} C ${x1} ${c1y}, ${x2} ${c2y}, ${x2} ${y2}`;
}

// ─── primitive renderers ───────────────────────────────────────────────────

function Region({
  region
}) {
  const accentVar = region.accent ? `var(--${region.accent})` : "var(--ink-3)";
  return /*#__PURE__*/React.createElement("div", {
    className: "region",
    "data-region-id": region.id,
    "data-region-ord": region.ord,
    "data-boundary": region.boundary || "",
    "data-entry": region.entry ? "true" : undefined,
    style: {
      left: region.x,
      top: region.y,
      width: region.w,
      height: region.h,
      "--region-accent": accentVar
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "region-label"
  }, /*#__PURE__*/React.createElement("span", {
    className: "region-ord"
  }, region.ord), /*#__PURE__*/React.createElement("span", {
    className: "region-title"
  }, region.title), /*#__PURE__*/React.createElement("span", {
    className: "region-caption"
  }, region.caption), region.lifetime && /*#__PURE__*/React.createElement("span", {
    className: "region-lifetime"
  }, "[", region.lifetime, "]")));
}
function Node({
  node,
  accent,
  highlighted,
  dimmed,
  traced,
  expanded,
  onHover,
  onLeave,
  onClick
}) {
  const cls = ["node", `kind-${node.kind}`, highlighted ? "is-hover" : "", dimmed ? "is-dim" : "", traced ? "is-traced" : "", expanded ? "is-expanded" : "", node.platform ? `platform-${node.platform}` : ""].join(" ");
  const accentVar = accent ? `var(--${accent})` : "var(--ink-3)";
  return /*#__PURE__*/React.createElement("div", {
    className: cls,
    "data-node-id": node.id,
    "data-region": node.region,
    style: {
      left: node.x,
      top: node.y,
      width: node.w,
      height: node.h,
      "--region-accent": accentVar
    },
    onMouseEnter: () => onHover(node.id),
    onMouseLeave: onLeave,
    onClick: () => onClick && onClick(node.id)
  }, /*#__PURE__*/React.createElement(NodeBody, {
    node: node
  }));
}
function NodeBody({
  node
}) {
  if (node.kind === "platform") {
    return /*#__PURE__*/React.createElement("div", {
      className: "platform-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "platform-header"
    }, /*#__PURE__*/React.createElement("span", {
      className: "platform-glyph"
    }, node.platform === "linux" ? "L" : node.platform === "macos" ? "M" : "W"), /*#__PURE__*/React.createElement("span", {
      className: "platform-name"
    }, node.label)), /*#__PURE__*/React.createElement("ul", {
      className: "platform-bullets"
    }, node.bullets.map((b, i) => /*#__PURE__*/React.createElement("li", {
      key: i
    }, b))), node.note && /*#__PURE__*/React.createElement("div", {
      className: "platform-note"
    }, node.note), node.code && /*#__PURE__*/React.createElement("div", {
      className: "node-code"
    }, node.code));
  }
  if (node.kind === "ext") {
    return /*#__PURE__*/React.createElement("div", {
      className: "ext-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "ext-row"
    }, /*#__PURE__*/React.createElement("span", {
      className: "ext-label"
    }, node.label), /*#__PURE__*/React.createElement("span", {
      className: "ext-pid"
    }, node.pid)), /*#__PURE__*/React.createElement("div", {
      className: "ext-sub"
    }, node.sub), node.internals && /*#__PURE__*/React.createElement("ul", {
      className: "ext-internals"
    }, node.internals.map((it, i) => /*#__PURE__*/React.createElement("li", {
      key: i
    }, it))), /*#__PURE__*/React.createElement("div", {
      className: "ext-meta"
    }, /*#__PURE__*/React.createElement("span", null, "plugin.toml"), /*#__PURE__*/React.createElement("span", {
      className: `ext-daemon ${node.daemon ? "on" : "off"}`
    }, node.daemon ? "▣ daemon" : "□ ephemeral")));
  }
  if (node.kind === "api") {
    return /*#__PURE__*/React.createElement("div", {
      className: "api-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "api-row"
    }, /*#__PURE__*/React.createElement("span", {
      className: "api-label"
    }, node.label), /*#__PURE__*/React.createElement("span", {
      className: "api-port"
    }, "127.0.0.1:42700")), /*#__PURE__*/React.createElement("div", {
      className: "api-sub"
    }, node.sub), node.ipc && /*#__PURE__*/React.createElement("div", {
      className: "api-ipc"
    }, node.ipc), node.code && /*#__PURE__*/React.createElement("div", {
      className: "node-code"
    }, node.code));
  }
  if (node.kind === "state") {
    return /*#__PURE__*/React.createElement("div", {
      className: "state-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "api-row"
    }, /*#__PURE__*/React.createElement("span", {
      className: "api-label"
    }, node.label), /*#__PURE__*/React.createElement("span", {
      className: "state-badge"
    }, "unix \xB7 read-only")), /*#__PURE__*/React.createElement("div", {
      className: "api-sub"
    }, node.sub), node.ipc && /*#__PURE__*/React.createElement("div", {
      className: "api-ipc"
    }, node.ipc), node.code && /*#__PURE__*/React.createElement("div", {
      className: "node-code"
    }, node.code));
  }
  if (node.kind === "router") {
    return /*#__PURE__*/React.createElement("div", {
      className: "router-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "api-row"
    }, /*#__PURE__*/React.createElement("span", {
      className: "api-label"
    }, node.label), /*#__PURE__*/React.createElement("span", {
      className: "router-badge"
    }, "OS thread")), /*#__PURE__*/React.createElement("div", {
      className: "api-sub"
    }, node.sub), node.ipc && /*#__PURE__*/React.createElement("div", {
      className: "api-ipc"
    }, node.ipc), node.code && /*#__PURE__*/React.createElement("div", {
      className: "node-code"
    }, node.code));
  }
  if (node.kind === "ps-file") {
    return /*#__PURE__*/React.createElement("div", {
      className: "psfile-card"
    }, /*#__PURE__*/React.createElement("div", {
      className: "psfile-head"
    }, /*#__PURE__*/React.createElement("span", {
      className: "psfile-label"
    }, node.label), node.env && /*#__PURE__*/React.createElement("span", {
      className: "psfile-env"
    }, "env: ", node.env)), /*#__PURE__*/React.createElement("div", {
      className: "psfile-path"
    }, node.path), /*#__PURE__*/React.createElement("div", {
      className: "psfile-rw"
    }, node.writes && node.writes.length > 0 && /*#__PURE__*/React.createElement("span", {
      className: "psfile-w",
      title: "written by"
    }, /*#__PURE__*/React.createElement("span", {
      className: "rw-glyph"
    }, "w"), node.writes.join(" · ")), node.reads && node.reads.length > 0 && /*#__PURE__*/React.createElement("span", {
      className: "psfile-r",
      title: "read by"
    }, /*#__PURE__*/React.createElement("span", {
      className: "rw-glyph"
    }, "r"), node.reads.join(" · "))));
  }
  if (node.kind === "store") {
    return /*#__PURE__*/React.createElement("div", {
      className: `store-card ${node.ephemeral ? "is-ephemeral" : ""}`
    }, /*#__PURE__*/React.createElement("div", {
      className: "store-row"
    }, /*#__PURE__*/React.createElement("span", {
      className: "generic-label"
    }, node.label), node.ephemeral && /*#__PURE__*/React.createElement("span", {
      className: "store-badge"
    }, "ephemeral")), node.sub && /*#__PURE__*/React.createElement("div", {
      className: "generic-sub"
    }, node.sub), node.path && /*#__PURE__*/React.createElement("div", {
      className: "store-path"
    }, node.path));
  }
  // core / generic — supports an optional bullets list (e.g. event bus variants),
  // a `note` field (small italic footer), and the `plug-anchor` variant for fan-in nodes.
  return /*#__PURE__*/React.createElement("div", {
    className: "generic"
  }, /*#__PURE__*/React.createElement("div", {
    className: "generic-label"
  }, node.label), node.sub && /*#__PURE__*/React.createElement("div", {
    className: "generic-sub"
  }, node.sub), node.bullets && /*#__PURE__*/React.createElement("ul", {
    className: "generic-bullets"
  }, node.bullets.map((b, i) => /*#__PURE__*/React.createElement("li", {
    key: i
  }, b))), node.note && /*#__PURE__*/React.createElement("div", {
    className: "generic-note"
  }, node.note), node.code && /*#__PURE__*/React.createElement("div", {
    className: "node-code"
  }, node.code));
}

// ─── band gutter chevrons ──────────────────────────────────────────────────

function Gutters() {
  const {
    GUTTERS,
    CANVAS
  } = window.QOL_DIAGRAM;
  const cx = CANVAS.w / 2;
  return /*#__PURE__*/React.createElement("svg", {
    className: "gutters",
    viewBox: `0 0 ${CANVAS.w} ${CANVAS.h}`,
    preserveAspectRatio: "none"
  }, GUTTERS.map((g, i) => {
    const mid = (g.fromY + g.toY) / 2;
    const top = g.fromY + 4;
    const bot = g.toY - 4;
    const cls = `gutter ${g.dashed ? "is-dashed" : ""} tone-${g.tone || "ink"}`;
    return /*#__PURE__*/React.createElement("g", {
      key: i,
      className: cls
    }, /*#__PURE__*/React.createElement("line", {
      x1: cx,
      y1: top,
      x2: cx,
      y2: bot - 8,
      className: "gutter-line"
    }), /*#__PURE__*/React.createElement("path", {
      d: `M ${cx - 6} ${bot - 10} L ${cx} ${bot} L ${cx + 6} ${bot - 10}`,
      className: "gutter-chev"
    }), /*#__PURE__*/React.createElement("text", {
      x: cx + 14,
      y: mid + 3,
      className: "gutter-label"
    }, g.label));
  }));
}

// ─── edges layer ───────────────────────────────────────────────────────────

function Edges({
  nodes,
  edges,
  tracedPairs,
  canvasH
}) {
  const {
    CANVAS
  } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  const nodeMap = useMemo(() => new Map(nodes.map(n => [n.id, n])), [nodes]);
  const staticPaths = useMemo(() => {
    return edges.map((edge, i) => {
      const from = nodeMap.get(edge.from);
      const to = nodeMap.get(edge.to);
      if (!from || !to) return null;
      const [autoFrom, autoTo] = autoSides(from, to);
      const fromSide = edge.fromSide || autoFrom;
      const toSide = edge.toSide || autoTo;
      const a = sidePoint(from, fromSide);
      const b = sidePoint(to, toSide);
      return {
        i,
        edge: {
          ...edge,
          fromSide,
          toSide
        },
        d: bezierPath(a, b, {
          ...edge,
          fromSide,
          toSide
        })
      };
    }).filter(Boolean);
  }, [edges, nodeMap]);

  // Compute trace overlay paths (built from active trace; not from EDGES).
  const tracePaths = useMemo(() => {
    if (!tracedPairs) return [];
    return tracedPairs.paths;
  }, [tracedPairs]);
  return /*#__PURE__*/React.createElement("svg", {
    className: "edges",
    viewBox: `0 0 ${CANVAS.w} ${h}`,
    preserveAspectRatio: "none"
  }, /*#__PURE__*/React.createElement("defs", null, /*#__PURE__*/React.createElement("marker", {
    id: "arrow-ink",
    viewBox: "0 0 10 10",
    refX: "9",
    refY: "5",
    markerWidth: "7",
    markerHeight: "7",
    orient: "auto-start-reverse"
  }, /*#__PURE__*/React.createElement("path", {
    d: "M 0 1 L 9 5 L 0 9 z",
    className: "arrow-ink"
  })), /*#__PURE__*/React.createElement("marker", {
    id: "arrow-amber",
    viewBox: "0 0 10 10",
    refX: "9",
    refY: "5",
    markerWidth: "7",
    markerHeight: "7",
    orient: "auto-start-reverse"
  }, /*#__PURE__*/React.createElement("path", {
    d: "M 0 1 L 9 5 L 0 9 z",
    className: "arrow-amber"
  })), /*#__PURE__*/React.createElement("marker", {
    id: "arrow-slate",
    viewBox: "0 0 10 10",
    refX: "9",
    refY: "5",
    markerWidth: "7",
    markerHeight: "7",
    orient: "auto-start-reverse"
  }, /*#__PURE__*/React.createElement("path", {
    d: "M 0 1 L 9 5 L 0 9 z",
    className: "arrow-slate"
  }))), staticPaths.map(({
    i,
    edge,
    d
  }) => {
    const cls = ["edge", `tone-${edge.tone || "ink"}`, edge.dashed ? "is-dashed" : "", edge.hairline ? "is-hairline" : "", edge.internal ? "is-internal" : "", edge.bypass ? "is-bypass" : "", edge.longRail ? "is-longrail" : "", tracedPairs ? "is-fade" : ""].join(" ");
    // Every edge gets an arrowhead — flow direction has to be readable
    // without context (Tufte: lines are multivocal; explicit terminators
    // pin down the semantic). Originally only internal/bypass/longRail
    // edges showed arrows; cross-region edges (spine) were left without,
    // which made minimal view read as a passive web rather than a flow.
    const showArrow = true;
    return /*#__PURE__*/React.createElement("path", {
      key: `s-${i}`,
      d: d,
      className: cls,
      markerEnd: showArrow ? `url(#arrow-${edge.tone || "ink"})` : ""
    });
  }), tracePaths.map((d, i) => /*#__PURE__*/React.createElement("path", {
    key: `t-${i}`,
    d: d,
    className: `edge trace-edge tone-${tracedPairs.tone}`,
    markerEnd: `url(#arrow-${tracedPairs.tone})`
  })));
}

// ─── main diagram ──────────────────────────────────────────────────────────

function Diagram({
  tweaks,
  setTweak
}) {
  const {
    REGIONS: SRC_REGIONS,
    NODES: SRC_NODES,
    TRACES,
    CANVAS
  } = window.QOL_DIAGRAM;
  const level = tweaks.level || "minimal";
  const compact = level === "minimal";
  const [hoverId, setHoverId] = useState(null);
  const [expandedIds, setExpandedIds] = useState(() => new Set());
  const [activeTrace, setActiveTrace] = useState(null);
  const [scale, setScale] = useState(1);
  const outerRef = useRef(null);
  const toggleExpand = useCallback(id => {
    setExpandedIds(prev => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);else next.add(id);
      return next;
    });
  }, []);

  // minimal      - tier-1 nodes only, row-packed, centered.
  // descriptive  - all nodes, row-packed in original columns, shorter cards.
  // detailed     - the source layout from data.js, all cards fully rendered.
  const layout = useMemo(() => {
    if (level === "minimal") return computeMinimalLayout(SRC_REGIONS, SRC_NODES);
    if (level === "descriptive") return computeDescriptiveLayout(SRC_REGIONS, SRC_NODES);
    // Detailed: source layout, but strip minimalOnly synthetics.
    return {
      regions: SRC_REGIONS,
      nodes: SRC_NODES.filter(n => !n.minimalOnly),
      canvasH: CANVAS.h
    };
  }, [level, SRC_REGIONS, SRC_NODES, CANVAS.h]);
  const REGIONS = layout.regions;
  const NODES = layout.nodes;
  const CANVAS_H = layout.canvasH;

  // Layout-aware node lookup. Both trace overlays and the edges layer need
  // positions from the current layout pass, not the source coords.
  const nodeMap = useMemo(() => new Map(NODES.map(n => [n.id, n])), [NODES]);

  // Minimal renders synthesized inter-region flow (META.minimalFlow);
  // descriptive and detailed render the full edge set from data.js.
  const SRC_EDGES = window.QOL_DIAGRAM.EDGES;
  const META = window.QOL_DIAGRAM.META || {};
  const visibleEdges = useMemo(() => {
    return level === "minimal" ? META.minimalFlow || [] : SRC_EDGES;
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
      const vw = window.innerWidth - pad * 2;
      const vh = window.innerHeight - topH - pad * 2;
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
    return () => {
      window.removeEventListener("resize", fit);
      clearTimeout(t);
    };
  }, [CANVAS.w, CANVAS_H]);

  // Trace state derivations.
  const tracedNodeIds = useMemo(() => activeTrace ? new Set(activeTrace.steps) : null, [activeTrace]);
  const tracedPairs = useMemo(() => {
    if (!activeTrace) return null;
    const paths = [];
    for (let i = 0; i < activeTrace.steps.length - 1; i++) {
      const a = nodeMap.get(activeTrace.steps[i]);
      const b = nodeMap.get(activeTrace.steps[i + 1]);
      if (!a || !b) continue;
      const [fromSide, toSide] = autoSides(a, b);
      const pa = sidePoint(a, fromSide);
      const pb = sidePoint(b, toSide);
      paths.push(bezierPath(pa, pb, {
        fromSide,
        toSide
      }));
    }
    return {
      paths,
      tone: activeTrace.tone || "ink"
    };
  }, [activeTrace, nodeMap]);
  const onSelectTrace = t => {
    // Trace nodes belong to the original layout, not the compact one,
    // so picking a trace while in minimal also expands the diagram.
    if (compact && setTweak) setTweak("level", "descriptive");
    setActiveTrace(prev => prev && prev.id === t.id ? null : t);
    setExpandedIds(new Set());
  };
  return /*#__PURE__*/React.createElement("div", {
    className: "diagram-root"
  }, /*#__PURE__*/React.createElement(Topbar, {
    activeTrace: activeTrace,
    traces: TRACES,
    onSelectTrace: onSelectTrace,
    level: tweaks.level || "minimal",
    setLevel: l => setTweak && setTweak("level", l)
  }), /*#__PURE__*/React.createElement("div", {
    className: "stage-outer",
    ref: outerRef
  }, /*#__PURE__*/React.createElement("div", {
    className: "stage-wrap",
    style: {
      width: CANVAS.w * scale,
      height: CANVAS_H * scale
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "stage",
    style: {
      width: CANVAS.w,
      height: CANVAS_H,
      transform: `scale(${scale})`
    }
  }, /*#__PURE__*/React.createElement(PaperBackdrop, null), REGIONS.map(r => /*#__PURE__*/React.createElement(Region, {
    key: r.id,
    region: r
  })), level !== "detailed" && /*#__PURE__*/React.createElement(FlowChevrons, {
    regions: REGIONS,
    canvasW: CANVAS.w,
    canvasH: CANVAS_H,
    accentByRegion: accentByRegion
  }), level === "detailed" && /*#__PURE__*/React.createElement(Quadrants, null), level === "detailed" && /*#__PURE__*/React.createElement(Gutters, null), level === "detailed" && /*#__PURE__*/React.createElement(TokioBoundary, null), level === "detailed" && /*#__PURE__*/React.createElement(LaneLabels, null), /*#__PURE__*/React.createElement(Edges, {
    nodes: NODES,
    edges: visibleEdges,
    tracedPairs: tracedPairs,
    canvasH: CANVAS_H
  }), NODES.map(n => {
    const dimmed = tracedNodeIds && !tracedNodeIds.has(n.id);
    const traced = tracedNodeIds && tracedNodeIds.has(n.id);
    const highlighted = hoverId === n.id;
    const expanded = expandedIds.has(n.id);
    return /*#__PURE__*/React.createElement(Node, {
      key: n.id,
      node: n,
      accent: accentByRegion.get(n.region),
      highlighted: highlighted,
      dimmed: dimmed,
      traced: traced,
      expanded: expanded,
      onHover: setHoverId,
      onLeave: () => setHoverId(null),
      onClick: toggleExpand
    });
  }), /*#__PURE__*/React.createElement(CornerMarks, {
    canvasH: CANVAS_H
  }), /*#__PURE__*/React.createElement(PlateAnnotations, {
    activeTrace: activeTrace,
    canvasH: CANVAS_H
  })))), /*#__PURE__*/React.createElement(DetailPanel, {
    activeTrace: activeTrace,
    onClose: () => setActiveTrace(null)
  }));
}

// Downward chevrons between consecutive regions - reads as a numbered flow
// path through r1 → r2 → ... → r6. Computed from the current layout so it
// follows minimal and descriptive compaction, not the source coords.
function FlowChevrons({
  regions,
  canvasW,
  canvasH,
  accentByRegion
}) {
  if (regions.length < 2) return null;
  const cx = canvasW / 2;
  return /*#__PURE__*/React.createElement("svg", {
    className: "flow-chevrons",
    viewBox: `0 0 ${canvasW} ${canvasH}`,
    preserveAspectRatio: "none"
  }, regions.slice(0, -1).map((r, i) => {
    const next = regions[i + 1];
    const cy = (r.y + r.h + next.y) / 2;
    const accent = accentByRegion && accentByRegion.get(r.id);
    const style = accent ? {
      "--region-accent": `var(--${accent})`
    } : undefined;
    return /*#__PURE__*/React.createElement("g", {
      key: i,
      className: "flow-chev",
      style: style
    }, /*#__PURE__*/React.createElement("path", {
      d: `M ${cx - 18} ${cy - 8} L ${cx} ${cy + 6} L ${cx + 18} ${cy - 8}`
    }));
  }));
}
function Quadrants() {
  const {
    META,
    CANVAS
  } = window.QOL_DIAGRAM;
  if (!META || !META.quadrants) return null;
  // Compute the cross-divider extents from the four quadrant bounds.
  const xs = META.quadrants.map(q => q.x);
  const ws = META.quadrants.map(q => q.x + q.w);
  const ys = META.quadrants.map(q => q.y);
  const hs = META.quadrants.map(q => q.y + q.h);
  const left = Math.min(...xs),
    right = Math.max(...ws);
  const top = Math.min(...ys),
    bot = Math.max(...hs);
  // Vertical divider between left & right halves.
  const midX = (Math.max(...xs.filter((x, i) => META.quadrants[i].id.endsWith("l"))) + Math.min(...xs.filter((x, i) => META.quadrants[i].id.endsWith("r")))) / 2;
  // Horizontal divider between top & bottom halves.
  const midY = (Math.max(...ys.filter((y, i) => META.quadrants[i].id.startsWith("t"))) + Math.min(...ys.filter((y, i) => META.quadrants[i].id.startsWith("b")))) / 2;
  return /*#__PURE__*/React.createElement(React.Fragment, null, META.quadrants.map(q => /*#__PURE__*/React.createElement("div", {
    key: q.id,
    className: `quadrant quadrant-${q.id}`,
    style: {
      left: q.x,
      top: q.y,
      width: q.w,
      height: q.h
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "quadrant-label"
  }, /*#__PURE__*/React.createElement("span", {
    className: "quadrant-glyph"
  }, q.glyph), /*#__PURE__*/React.createElement("span", {
    className: "quadrant-ord"
  }, q.ord), /*#__PURE__*/React.createElement("span", {
    className: "quadrant-title"
  }, q.title), /*#__PURE__*/React.createElement("span", {
    className: "quadrant-axes"
  }, /*#__PURE__*/React.createElement("span", null, q.axisX), /*#__PURE__*/React.createElement("span", {
    className: "axes-dot"
  }, "\xB7"), /*#__PURE__*/React.createElement("span", null, q.axisY))))), /*#__PURE__*/React.createElement("svg", {
    className: "quadrant-cross",
    viewBox: `0 0 ${CANVAS.w} ${CANVAS.h}`,
    preserveAspectRatio: "none"
  }, /*#__PURE__*/React.createElement("line", {
    x1: midX,
    y1: top,
    x2: midX,
    y2: bot,
    className: "qcross-line"
  }), /*#__PURE__*/React.createElement("line", {
    x1: left,
    y1: midY,
    x2: right,
    y2: midY,
    className: "qcross-line"
  })));
}
function TokioBoundary() {
  const {
    META,
    REGIONS
  } = window.QOL_DIAGRAM;
  if (!META || !META.tokioBoundary) return null;
  const r3 = REGIONS.find(r => r.id === "r3");
  if (!r3) return null;
  const y = META.tokioBoundary.y;
  const left = r3.x + 12;
  const right = r3.x + r3.w - 12;
  return /*#__PURE__*/React.createElement("svg", {
    className: "tokio-boundary",
    viewBox: `0 0 ${window.QOL_DIAGRAM.CANVAS.w} ${window.QOL_DIAGRAM.CANVAS.h}`,
    preserveAspectRatio: "none"
  }, /*#__PURE__*/React.createElement("line", {
    x1: left,
    y1: y,
    x2: right,
    y2: y,
    className: "tokio-rule"
  }), /*#__PURE__*/React.createElement("line", {
    x1: left,
    y1: y - 6,
    x2: left,
    y2: y + 6,
    className: "tokio-tick"
  }), /*#__PURE__*/React.createElement("line", {
    x1: right,
    y1: y - 6,
    x2: right,
    y2: y + 6,
    className: "tokio-tick"
  }), /*#__PURE__*/React.createElement("rect", {
    x: (left + right) / 2 - 110,
    y: y - 11,
    width: "220",
    height: "22",
    className: "tokio-label-bg"
  }), /*#__PURE__*/React.createElement("text", {
    x: (left + right) / 2,
    y: y + 4,
    className: "tokio-label-text"
  }, "\u2193  tokio multi-thread runtime  \u2193"), /*#__PURE__*/React.createElement("text", {
    x: left + 6,
    y: y - 12,
    className: "tokio-phase-text"
  }, "pre-tokio \xB7 main thread"), /*#__PURE__*/React.createElement("text", {
    x: right - 6,
    y: y - 12,
    className: "tokio-phase-text",
    textAnchor: "end"
  }, "async tasks"));
}
function LaneLabels() {
  const {
    META,
    CANVAS
  } = window.QOL_DIAGRAM;
  if (!META || !META.laneLabels) return null;
  return /*#__PURE__*/React.createElement("svg", {
    className: "lane-labels",
    viewBox: `0 0 ${CANVAS.w} ${CANVAS.h}`,
    preserveAspectRatio: "none"
  }, META.laneLabels.map((l, i) => /*#__PURE__*/React.createElement("g", {
    key: i,
    transform: `translate(${l.x}, ${l.y + l.h / 2})`
  }, /*#__PURE__*/React.createElement("text", {
    className: "lane-label-text",
    transform: "rotate(-90)"
  }, l.label))));
}
function PaperBackdrop() {
  return /*#__PURE__*/React.createElement("div", {
    className: "paper-backdrop",
    "aria-hidden": "true"
  });
}
function CornerMarks({
  canvasH
}) {
  const {
    CANVAS
  } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  const mark = (x, y, dx, dy) => /*#__PURE__*/React.createElement("g", null, /*#__PURE__*/React.createElement("line", {
    x1: x,
    y1: y,
    x2: x + dx,
    y2: y,
    className: "mark"
  }), /*#__PURE__*/React.createElement("line", {
    x1: x,
    y1: y,
    x2: x,
    y2: y + dy,
    className: "mark"
  }));
  return /*#__PURE__*/React.createElement("svg", {
    className: "corner-marks",
    viewBox: `0 0 ${CANVAS.w} ${h}`
  }, mark(8, 8, 24, 24), mark(CANVAS.w - 8, 8, -24, 24), mark(8, h - 8, 24, -24), mark(CANVAS.w - 8, h - 8, -24, -24));
}
function PlateAnnotations({
  activeTrace,
  canvasH
}) {
  const {
    CANVAS,
    META
  } = window.QOL_DIAGRAM;
  const h = canvasH != null ? canvasH : CANVAS.h;
  return /*#__PURE__*/React.createElement("div", {
    className: "plate-annotations",
    style: {
      width: CANVAS.w,
      height: h
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "plate plate-tl"
  }, /*#__PURE__*/React.createElement("div", {
    className: "plate-line"
  }, "fig \xB7 01"), /*#__PURE__*/React.createElement("div", {
    className: "plate-line plate-mono"
  }, "runtime architecture map")), /*#__PURE__*/React.createElement("div", {
    className: "plate plate-tr"
  }, /*#__PURE__*/React.createElement("div", {
    className: "plate-line"
  }, "qol-tray"), /*#__PURE__*/React.createElement("div", {
    className: "plate-line plate-mono"
  }, "v3.15.1 \xB7 main")), /*#__PURE__*/React.createElement("div", {
    className: "plate plate-bl plate-mono"
  }, META && META.binaries), /*#__PURE__*/React.createElement("div", {
    className: "plate plate-br plate-mono"
  }, activeTrace ? `trace · ${activeTrace.ord} · ${activeTrace.label}` : "trace · idle"));
}

// ─── topbar ────────────────────────────────────────────────────────────────

function Topbar({
  activeTrace,
  traces,
  onSelectTrace,
  level,
  setLevel
}) {
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
  return /*#__PURE__*/React.createElement("div", {
    className: "topbar"
  }, /*#__PURE__*/React.createElement("div", {
    className: "topbar-left"
  }, /*#__PURE__*/React.createElement("span", {
    className: "topbar-title"
  }, "QoL Tray"), /*#__PURE__*/React.createElement("span", {
    className: "topbar-sep"
  }, "\xB7"), /*#__PURE__*/React.createElement("span", {
    className: "topbar-sub"
  }, "Runtime Architecture Map")), /*#__PURE__*/React.createElement("div", {
    className: "topbar-center",
    ref: menuRef
  }, /*#__PURE__*/React.createElement("button", {
    className: `trace-toggle ${activeTrace ? "is-active" : ""}`,
    onClick: () => setTraceOpen(o => !o)
  }, /*#__PURE__*/React.createElement("span", {
    className: "trace-toggle-ico"
  }, "\u25B7"), /*#__PURE__*/React.createElement("span", {
    className: "trace-toggle-label"
  }, activeTrace ? /*#__PURE__*/React.createElement(React.Fragment, null, "trace \xB7 ", /*#__PURE__*/React.createElement("b", null, activeTrace.ord), " ", activeTrace.label) : "traces"), activeTrace ? /*#__PURE__*/React.createElement("span", {
    className: "trace-toggle-stop",
    onClick: e => {
      e.stopPropagation();
      onSelectTrace(activeTrace);
    }
  }, "\xD7") : /*#__PURE__*/React.createElement("span", {
    className: "trace-toggle-caret"
  }, traceOpen ? "▴" : "▾")), traceOpen && /*#__PURE__*/React.createElement("div", {
    className: "trace-menu"
  }, /*#__PURE__*/React.createElement("div", {
    className: "trace-menu-eyebrow"
  }, "runtime trace"), traces.map(t => /*#__PURE__*/React.createElement("button", {
    key: t.id,
    className: `trace-menu-item ${activeTrace && activeTrace.id === t.id ? "is-on" : ""} ${t.tone === "amber" ? "tone-amber" : ""}`,
    onClick: () => {
      onSelectTrace(t);
      setTraceOpen(false);
    }
  }, /*#__PURE__*/React.createElement("span", {
    className: "trace-ord"
  }, t.ord), /*#__PURE__*/React.createElement("span", {
    className: "trace-label"
  }, t.label), /*#__PURE__*/React.createElement("span", {
    className: "trace-play"
  }, activeTrace && activeTrace.id === t.id ? "■" : "▷"))))), /*#__PURE__*/React.createElement("div", {
    className: "topbar-right"
  }, /*#__PURE__*/React.createElement("span", {
    className: "topbar-eyebrow"
  }, "level"), ["minimal", "descriptive", "detailed"].map(l => /*#__PURE__*/React.createElement("button", {
    key: l,
    className: `pill ${level === l ? "is-on" : ""}`,
    onClick: () => setLevel(l)
  }, l))));
}

// ─── detail panel ──────────────────────────────────────────────────────────

// Trace narrative side panel. Node details are now shown inline by clicking
// the card to toggle expansion, so this panel is trace-only.
function DetailPanel({
  activeTrace,
  onClose
}) {
  if (!activeTrace) return null;
  return /*#__PURE__*/React.createElement("div", {
    className: "detail-panel"
  }, /*#__PURE__*/React.createElement("button", {
    className: "detail-close",
    onClick: onClose,
    "aria-label": "close"
  }, "\xD7"), /*#__PURE__*/React.createElement("div", {
    className: "detail-section"
  }, /*#__PURE__*/React.createElement("div", {
    className: "detail-eyebrow"
  }, "trace \xB7 ", activeTrace.ord), /*#__PURE__*/React.createElement("div", {
    className: "detail-title"
  }, activeTrace.label), /*#__PURE__*/React.createElement("div", {
    className: "detail-body"
  }, activeTrace.narrative), /*#__PURE__*/React.createElement("div", {
    className: "detail-trace-steps"
  }, activeTrace.steps.map((id, i) => {
    const n = nodeById(id);
    return /*#__PURE__*/React.createElement(React.Fragment, {
      key: i
    }, /*#__PURE__*/React.createElement("span", {
      className: "step"
    }, n ? n.label : id), i < activeTrace.steps.length - 1 && /*#__PURE__*/React.createElement("span", {
      className: "step-sep"
    }, "\u2193"));
  }))));
}
window.Diagram = Diagram;
//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJuYW1lcyI6WyJ1c2VTdGF0ZSIsInVzZUVmZmVjdCIsInVzZVJlZiIsInVzZU1lbW8iLCJ1c2VDYWxsYmFjayIsIlJlYWN0Iiwibm9kZUJ5SWQiLCJpZCIsIndpbmRvdyIsIlFPTF9ESUFHUkFNIiwiTk9ERVMiLCJmaW5kIiwibiIsInJlYWRNZXRhIiwiTUVUQSIsImF1dG9TaWRlcyIsImEiLCJiIiwiYUN5IiwieSIsImgiLCJiQ3kiLCJNYXRoIiwiYWJzIiwibWF4IiwieCIsImdyb3VwUm93cyIsIm5vZGVzIiwic29ydGVkIiwic29ydCIsInJvd3MiLCJsYXN0IiwibGVuZ3RoIiwicHVzaCIsImxhc3RDIiwibkMiLCJ0b2wiLCJjb21wdXRlQ29tcGFjdExheW91dCIsInNyY1JlZ2lvbnMiLCJzcmNOb2RlcyIsIm9wdHMiLCJDQU5WQVMiLCJyZXNpemUiLCJoZWlnaHRUYWJsZSIsImtpbmQiLCJ3Iiwid2lkdGhUYWJsZSIsInNpemVkIiwibm9kZUZpbHRlciIsImZpbHRlciIsIm1hcCIsImJ5UmVnaW9uIiwiTWFwIiwiaGFzIiwicmVnaW9uIiwic2V0IiwiZ2V0IiwiTEFCRUxfUEFEX1RPUCIsIlBBRF9CT1RUT00iLCJUT1BfUEFEIiwiRU1QVFlfUkVHSU9OX0giLCJjdXJzb3IiLCJyZWdpb25zIiwiciIsImhlcmUiLCJyZWdpb25HdXR0ZXIiLCJyb3dZIiwibGFzdEJvdHRvbSIsInJvdyIsInJvd0giLCJjZW50ZXJSb3dzIiwiSEciLCJ0b3RhbFciLCJyZWR1Y2UiLCJzIiwiY3giLCJyb3dHYXAiLCJyZWdpb25IIiwiY2FudmFzSCIsImJ1aWxkU2l6ZVRhYmxlIiwia2luZFN0eWxlcyIsImhLZXkiLCJ3S2V5IiwiT2JqZWN0IiwiZW50cmllcyIsInVuZGVmaW5lZCIsImNvbXB1dGVNaW5pbWFsTGF5b3V0IiwibWV0YSIsInRpZXIxIiwiU2V0IiwiY29tcHV0ZURlc2NyaXB0aXZlTGF5b3V0IiwibWluaW1hbE9ubHkiLCJTSURFX0dBUCIsInNpZGVQb2ludCIsIm5vZGUiLCJzaWRlIiwib3J0aG9nb25hbFBhdGgiLCJ4MSIsInkxIiwieDIiLCJ5MiIsImZyb21TaWRlIiwidG9TaWRlIiwibWlkWSIsInNnbiIsImNyIiwibWluIiwiYmV6aWVyUGF0aCIsImVkZ2UiLCJyb3V0ZSIsImhvcml6IiwiYnlwYXNzIiwicmFpbFgiLCJsb25nUmFpbCIsImsiLCJnb0Rvd24iLCJyYWlsRW50cnlZMSIsInJhaWxFbnRyeVkyIiwid3JhcCIsInJhaWxZIiwiZHJvcFgiLCJzaWRlTG9vcFJldHVybiIsImFyY1kiLCJkeCIsImMxeCIsImMyeCIsImR5IiwiYzF5IiwiYzJ5IiwiUmVnaW9uIiwiYWNjZW50VmFyIiwiYWNjZW50IiwiY3JlYXRlRWxlbWVudCIsImNsYXNzTmFtZSIsIm9yZCIsImJvdW5kYXJ5IiwiZW50cnkiLCJzdHlsZSIsImxlZnQiLCJ0b3AiLCJ3aWR0aCIsImhlaWdodCIsInRpdGxlIiwiY2FwdGlvbiIsImxpZmV0aW1lIiwiTm9kZSIsImhpZ2hsaWdodGVkIiwiZGltbWVkIiwidHJhY2VkIiwiZXhwYW5kZWQiLCJvbkhvdmVyIiwib25MZWF2ZSIsIm9uQ2xpY2siLCJjbHMiLCJwbGF0Zm9ybSIsImpvaW4iLCJvbk1vdXNlRW50ZXIiLCJvbk1vdXNlTGVhdmUiLCJOb2RlQm9keSIsImxhYmVsIiwiYnVsbGV0cyIsImkiLCJrZXkiLCJub3RlIiwiY29kZSIsInBpZCIsInN1YiIsImludGVybmFscyIsIml0IiwiZGFlbW9uIiwiaXBjIiwiZW52IiwicGF0aCIsIndyaXRlcyIsInJlYWRzIiwiZXBoZW1lcmFsIiwiR3V0dGVycyIsIkdVVFRFUlMiLCJ2aWV3Qm94IiwicHJlc2VydmVBc3BlY3RSYXRpbyIsImciLCJtaWQiLCJmcm9tWSIsInRvWSIsImJvdCIsImRhc2hlZCIsInRvbmUiLCJkIiwiRWRnZXMiLCJlZGdlcyIsInRyYWNlZFBhaXJzIiwibm9kZU1hcCIsInN0YXRpY1BhdGhzIiwiZnJvbSIsInRvIiwiYXV0b0Zyb20iLCJhdXRvVG8iLCJCb29sZWFuIiwidHJhY2VQYXRocyIsInBhdGhzIiwicmVmWCIsInJlZlkiLCJtYXJrZXJXaWR0aCIsIm1hcmtlckhlaWdodCIsIm9yaWVudCIsImhhaXJsaW5lIiwiaW50ZXJuYWwiLCJzaG93QXJyb3ciLCJtYXJrZXJFbmQiLCJEaWFncmFtIiwidHdlYWtzIiwic2V0VHdlYWsiLCJSRUdJT05TIiwiU1JDX1JFR0lPTlMiLCJTUkNfTk9ERVMiLCJUUkFDRVMiLCJsZXZlbCIsImNvbXBhY3QiLCJob3ZlcklkIiwic2V0SG92ZXJJZCIsImV4cGFuZGVkSWRzIiwic2V0RXhwYW5kZWRJZHMiLCJhY3RpdmVUcmFjZSIsInNldEFjdGl2ZVRyYWNlIiwic2NhbGUiLCJzZXRTY2FsZSIsIm91dGVyUmVmIiwidG9nZ2xlRXhwYW5kIiwicHJldiIsIm5leHQiLCJkZWxldGUiLCJhZGQiLCJsYXlvdXQiLCJDQU5WQVNfSCIsIlNSQ19FREdFUyIsIkVER0VTIiwidmlzaWJsZUVkZ2VzIiwibWluaW1hbEZsb3ciLCJhY2NlbnRCeVJlZ2lvbiIsIm0iLCJmaXQiLCJ0b3BiYXIiLCJkb2N1bWVudCIsInF1ZXJ5U2VsZWN0b3IiLCJ0b3BIIiwiZ2V0Qm91bmRpbmdDbGllbnRSZWN0IiwicGFkIiwidnciLCJpbm5lcldpZHRoIiwidmgiLCJpbm5lckhlaWdodCIsImZpdHNIZWlnaHQiLCJhZGRFdmVudExpc3RlbmVyIiwidCIsInNldFRpbWVvdXQiLCJyZW1vdmVFdmVudExpc3RlbmVyIiwiY2xlYXJUaW1lb3V0IiwidHJhY2VkTm9kZUlkcyIsInN0ZXBzIiwicGEiLCJwYiIsIm9uU2VsZWN0VHJhY2UiLCJUb3BiYXIiLCJ0cmFjZXMiLCJzZXRMZXZlbCIsImwiLCJyZWYiLCJ0cmFuc2Zvcm0iLCJQYXBlckJhY2tkcm9wIiwiRmxvd0NoZXZyb25zIiwiY2FudmFzVyIsIlF1YWRyYW50cyIsIlRva2lvQm91bmRhcnkiLCJMYW5lTGFiZWxzIiwiQ29ybmVyTWFya3MiLCJQbGF0ZUFubm90YXRpb25zIiwiRGV0YWlsUGFuZWwiLCJvbkNsb3NlIiwic2xpY2UiLCJjeSIsInF1YWRyYW50cyIsInhzIiwicSIsIndzIiwieXMiLCJocyIsInJpZ2h0IiwibWlkWCIsImVuZHNXaXRoIiwic3RhcnRzV2l0aCIsIkZyYWdtZW50IiwiZ2x5cGgiLCJheGlzWCIsImF4aXNZIiwidG9raW9Cb3VuZGFyeSIsInIzIiwidGV4dEFuY2hvciIsImxhbmVMYWJlbHMiLCJtYXJrIiwiYmluYXJpZXMiLCJ0cmFjZU9wZW4iLCJzZXRUcmFjZU9wZW4iLCJtZW51UmVmIiwib25Eb2NDbGljayIsImUiLCJjdXJyZW50IiwiY29udGFpbnMiLCJ0YXJnZXQiLCJvIiwic3RvcFByb3BhZ2F0aW9uIiwibmFycmF0aXZlIl0sInNvdXJjZXMiOlsiZGlhZ3JhbS5qc3giXSwic291cmNlc0NvbnRlbnQiOlsiLy8gRGlhZ3JhbSBmb3IgdGhlIFFvTCBUcmF5IFJ1bnRpbWUgQXJjaGl0ZWN0dXJlIE1hcCAodG9wLWRvd24pLlxuLy8gVmVydGljYWwgYmFuZHMgc3RhY2tlZCBvbiBhIDEzMjAgw5cgMTk2MCBjYW52YXMuIFN0YWdlIHNjYWxlcyB0byBmaXQgdGhlXG4vLyB2aWV3cG9ydCBXSURUSCAoY2FwcGVkIGF0IDHDlykgYW5kIHNjcm9sbHMgdmVydGljYWxseSDigJQgbm8gbGV0dGVyYm94LWNydXNoLlxuXG5jb25zdCB7IHVzZVN0YXRlLCB1c2VFZmZlY3QsIHVzZVJlZiwgdXNlTWVtbywgdXNlQ2FsbGJhY2sgfSA9IFJlYWN0O1xuXG4vLyDilIDilIDilIAgZ2VvbWV0cnkgaGVscGVycyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIBcblxuZnVuY3Rpb24gbm9kZUJ5SWQoaWQpIHsgcmV0dXJuIHdpbmRvdy5RT0xfRElBR1JBTS5OT0RFUy5maW5kKChuKSA9PiBuLmlkID09PSBpZCk7IH1cblxuLy8gUHJlc2VudGF0aW9uIHBvbGljeSBub3cgbGl2ZXMgaW4gZGF0YS5qcyB1bmRlciBNRVRBICh0aWVyMSwgbWluaW1hbEZsb3csXG4vLyBraW5kU3R5bGVzKS4gUmVzb2x2ZSBpdCBsYXppbHkgaW5zaWRlIHRoZSBsYXlvdXQgaGVscGVycyBzbyBlZGl0aW5nXG4vLyBkYXRhLmpzICsgcmVmcmVzaGluZyB0aGUgcGFnZSBpcyBlbm91Z2ggLSBubyBidWlsZCwgbm8ganN4IGVkaXQuXG5mdW5jdGlvbiByZWFkTWV0YSgpIHtcbiAgcmV0dXJuICh3aW5kb3cuUU9MX0RJQUdSQU0gJiYgd2luZG93LlFPTF9ESUFHUkFNLk1FVEEpIHx8IHt9O1xufVxuXG4vLyBQaWNrIGNvbm5lY3Rpb24gc2lkZXMgYXV0b21hdGljYWxseTogdmVydGljYWwgd2hlbiBZLWNlbnRlcnMgZGlmZmVyXG4vLyBzdWJzdGFudGlhbGx5LCBob3Jpem9udGFsIHdoZW4gdGhleSdyZSByb3VnaGx5IG9uIHRoZSBzYW1lIHJvdy5cbmZ1bmN0aW9uIGF1dG9TaWRlcyhhLCBiKSB7XG4gIGNvbnN0IGFDeSA9IGEueSArIGEuaCAvIDI7XG4gIGNvbnN0IGJDeSA9IGIueSArIGIuaCAvIDI7XG4gIGlmIChNYXRoLmFicyhhQ3kgLSBiQ3kpID4gTWF0aC5tYXgoYS5oLCBiLmgpIC8gMikge1xuICAgIHJldHVybiBiQ3kgPiBhQ3kgPyBbXCJib3R0b21cIiwgXCJ0b3BcIl0gOiBbXCJ0b3BcIiwgXCJib3R0b21cIl07XG4gIH1cbiAgcmV0dXJuIGIueCA+IGEueCA/IFtcInJpZ2h0XCIsIFwibGVmdFwiXSA6IFtcImxlZnRcIiwgXCJyaWdodFwiXTtcbn1cblxuLy8gR3JvdXAgbm9kZXMgaW50byByb3dzIGJ5IFktY2VudGVyIHByb3hpbWl0eSAod2l0aGluIG1heChoKS8yKS5cbmZ1bmN0aW9uIGdyb3VwUm93cyhub2Rlcykge1xuICBjb25zdCBzb3J0ZWQgPSBbLi4ubm9kZXNdLnNvcnQoKGEsIGIpID0+IGEueSAtIGIueSk7XG4gIGNvbnN0IHJvd3MgPSBbXTtcbiAgZm9yIChjb25zdCBuIG9mIHNvcnRlZCkge1xuICAgIGNvbnN0IGxhc3QgPSByb3dzW3Jvd3MubGVuZ3RoIC0gMV07XG4gICAgaWYgKCFsYXN0KSB7IHJvd3MucHVzaChbbl0pOyBjb250aW51ZTsgfVxuICAgIGNvbnN0IGxhc3RDID0gbGFzdFswXS55ICsgbGFzdFswXS5oIC8gMjtcbiAgICBjb25zdCBuQyAgICA9IG4ueSArIG4uaCAvIDI7XG4gICAgY29uc3QgdG9sICAgPSBNYXRoLm1heChsYXN0WzBdLmgsIG4uaCkgLyAyO1xuICAgIGlmIChNYXRoLmFicyhsYXN0QyAtIG5DKSA8IHRvbCkgbGFzdC5wdXNoKG4pO1xuICAgIGVsc2Ugcm93cy5wdXNoKFtuXSk7XG4gIH1cbiAgcmV0dXJuIHJvd3M7XG59XG5cbi8vIFNoYXJlZCBjb21wYWN0IGxheW91dCBwYXNzOiBncm91cHMgbm9kZXMgcGVyIHJlZ2lvbiBpbnRvIHJvd3MgYnlcbi8vIFktcHJveGltaXR5LCBkcm9wcyBvcmlnaW5hbCB2ZXJ0aWNhbCBnYXBzLCBvcHRpb25hbGx5IGNlbnRlcnMgZWFjaCByb3cuXG4vLyBCb3RoIHRoZSBtaW5pbWFsIGFuZCBkZXNjcmlwdGl2ZSBsZXZlbHMgZ28gdGhyb3VnaCB0aGlzOyBvbmx5IHRoZSBpbnB1dFxuLy8gZmlsdGVyaW5nLCBoZWlnaHQgdGFibGUsIGFuZCBnYXAgc2l6aW5nIGRpZmZlci5cbmZ1bmN0aW9uIGNvbXB1dGVDb21wYWN0TGF5b3V0KHNyY1JlZ2lvbnMsIHNyY05vZGVzLCBvcHRzKSB7XG4gIGNvbnN0IHsgQ0FOVkFTIH0gPSB3aW5kb3cuUU9MX0RJQUdSQU07XG4gIGNvbnN0IHJlc2l6ZSA9IChuKSA9PiAoe1xuICAgIC4uLm4sXG4gICAgaDogb3B0cy5oZWlnaHRUYWJsZVtuLmtpbmRdIHx8IG4uaCxcbiAgICB3OiBvcHRzLndpZHRoVGFibGUgPyAob3B0cy53aWR0aFRhYmxlW24ua2luZF0gfHwgbi53KSA6IG4udyxcbiAgfSk7XG4gIGNvbnN0IHNpemVkID0gb3B0cy5ub2RlRmlsdGVyXG4gICAgPyBzcmNOb2Rlcy5maWx0ZXIob3B0cy5ub2RlRmlsdGVyKS5tYXAocmVzaXplKVxuICAgIDogc3JjTm9kZXMubWFwKHJlc2l6ZSk7XG5cbiAgY29uc3QgYnlSZWdpb24gPSBuZXcgTWFwKCk7XG4gIGZvciAoY29uc3QgbiBvZiBzaXplZCkge1xuICAgIGlmICghYnlSZWdpb24uaGFzKG4ucmVnaW9uKSkgYnlSZWdpb24uc2V0KG4ucmVnaW9uLCBbXSk7XG4gICAgYnlSZWdpb24uZ2V0KG4ucmVnaW9uKS5wdXNoKG4pO1xuICB9XG5cbiAgY29uc3QgTEFCRUxfUEFEX1RPUCAgPSAzNjtcbiAgY29uc3QgUEFEX0JPVFRPTSAgICAgPSAyMDtcbiAgY29uc3QgVE9QX1BBRCAgICAgICAgPSA3MDtcbiAgY29uc3QgRU1QVFlfUkVHSU9OX0ggPSBMQUJFTF9QQURfVE9QICsgUEFEX0JPVFRPTTtcblxuICBsZXQgY3Vyc29yID0gVE9QX1BBRDtcbiAgY29uc3QgcmVnaW9ucyA9IFtdO1xuICBjb25zdCBub2RlcyA9IFtdO1xuXG4gIGZvciAoY29uc3QgciBvZiBzcmNSZWdpb25zKSB7XG4gICAgY29uc3QgaGVyZSA9IGJ5UmVnaW9uLmdldChyLmlkKSB8fCBbXTtcbiAgICBpZiAoaGVyZS5sZW5ndGggPT09IDApIHtcbiAgICAgIHJlZ2lvbnMucHVzaCh7IC4uLnIsIHk6IGN1cnNvciwgaDogRU1QVFlfUkVHSU9OX0ggfSk7XG4gICAgICBjdXJzb3IgKz0gRU1QVFlfUkVHSU9OX0ggKyBvcHRzLnJlZ2lvbkd1dHRlcjtcbiAgICAgIGNvbnRpbnVlO1xuICAgIH1cblxuICAgIGNvbnN0IHJvd3MgPSBncm91cFJvd3MoaGVyZSk7XG4gICAgbGV0IHJvd1kgPSBjdXJzb3IgKyBMQUJFTF9QQURfVE9QO1xuICAgIGxldCBsYXN0Qm90dG9tID0gcm93WTtcbiAgICBmb3IgKGNvbnN0IHJvdyBvZiByb3dzKSB7XG4gICAgICBjb25zdCByb3dIID0gTWF0aC5tYXgoLi4ucm93Lm1hcCgobikgPT4gbi5oKSk7XG4gICAgICBpZiAob3B0cy5jZW50ZXJSb3dzKSB7XG4gICAgICAgIC8vIFJlcGFjayBob3Jpem9udGFsbHkgd2l0aCBhIGZpeGVkIGdhcCBhbmQgY2VudGVyIHRoZSByb3cuIFNvdXJjZVxuICAgICAgICAvLyBnYXBzIGdyb3cgZGlzcHJvcG9ydGlvbmF0ZWx5IG9uY2Ugd2lkdGhzIHNocmluaywgc28gY2FyZHMgZHJpZnRcbiAgICAgICAgLy8gYXBhcnQgd2l0aG91dCB0aGlzIHJlcGFjay5cbiAgICAgICAgcm93LnNvcnQoKGEsIGIpID0+IGEueCAtIGIueCk7XG4gICAgICAgIGNvbnN0IEhHID0gMjQ7XG4gICAgICAgIGNvbnN0IHRvdGFsVyA9IHJvdy5yZWR1Y2UoKHMsIG4pID0+IHMgKyBuLncsIDApICsgSEcgKiAocm93Lmxlbmd0aCAtIDEpO1xuICAgICAgICBsZXQgY3ggPSAoQ0FOVkFTLncgLSB0b3RhbFcpIC8gMjtcbiAgICAgICAgZm9yIChjb25zdCBuIG9mIHJvdykge1xuICAgICAgICAgIG5vZGVzLnB1c2goeyAuLi5uLCB4OiBjeCwgeTogcm93WSB9KTtcbiAgICAgICAgICBjeCArPSBuLncgKyBIRztcbiAgICAgICAgfVxuICAgICAgfSBlbHNlIHtcbiAgICAgICAgZm9yIChjb25zdCBuIG9mIHJvdykgbm9kZXMucHVzaCh7IC4uLm4sIHk6IHJvd1kgfSk7XG4gICAgICB9XG4gICAgICBsYXN0Qm90dG9tID0gcm93WSArIHJvd0g7XG4gICAgICByb3dZICAgICAgID0gbGFzdEJvdHRvbSArIG9wdHMucm93R2FwO1xuICAgIH1cblxuICAgIGNvbnN0IHJlZ2lvbkggPSAobGFzdEJvdHRvbSArIFBBRF9CT1RUT00pIC0gY3Vyc29yO1xuICAgIHJlZ2lvbnMucHVzaCh7IC4uLnIsIHk6IGN1cnNvciwgaDogcmVnaW9uSCB9KTtcbiAgICBjdXJzb3IgKz0gcmVnaW9uSCArIG9wdHMucmVnaW9uR3V0dGVyO1xuICB9XG5cbiAgcmV0dXJuIHsgcmVnaW9ucywgbm9kZXMsIGNhbnZhc0g6IGN1cnNvciB9O1xufVxuXG5mdW5jdGlvbiBidWlsZFNpemVUYWJsZShraW5kU3R5bGVzLCBoS2V5LCB3S2V5KSB7XG4gIGNvbnN0IGggPSB7fSwgdyA9IHt9O1xuICBmb3IgKGNvbnN0IFtraW5kLCBzXSBvZiBPYmplY3QuZW50cmllcyhraW5kU3R5bGVzIHx8IHt9KSkge1xuICAgIGlmIChoS2V5ICYmIHNbaEtleV0gIT0gbnVsbCkgaFtraW5kXSA9IHNbaEtleV07XG4gICAgaWYgKHdLZXkgJiYgc1t3S2V5XSAhPSBudWxsKSB3W2tpbmRdID0gc1t3S2V5XTtcbiAgfVxuICByZXR1cm4geyBoZWlnaHRUYWJsZTogaCwgd2lkdGhUYWJsZTogd0tleSA/IHcgOiB1bmRlZmluZWQgfTtcbn1cblxuZnVuY3Rpb24gY29tcHV0ZU1pbmltYWxMYXlvdXQoc3JjUmVnaW9ucywgc3JjTm9kZXMpIHtcbiAgY29uc3QgbWV0YSA9IHJlYWRNZXRhKCk7XG4gIGNvbnN0IHRpZXIxID0gbmV3IFNldChtZXRhLnRpZXIxIHx8IFtdKTtcbiAgY29uc3QgeyBoZWlnaHRUYWJsZSwgd2lkdGhUYWJsZSB9ID0gYnVpbGRTaXplVGFibGUobWV0YS5raW5kU3R5bGVzLCBcIm1pbmltYWxIXCIsIFwibWluaW1hbFdcIik7XG4gIHJldHVybiBjb21wdXRlQ29tcGFjdExheW91dChzcmNSZWdpb25zLCBzcmNOb2Rlcywge1xuICAgIG5vZGVGaWx0ZXI6ICAgKG4pID0+IHRpZXIxLmhhcyhuLmlkKSxcbiAgICBoZWlnaHRUYWJsZSxcbiAgICB3aWR0aFRhYmxlLFxuICAgIGNlbnRlclJvd3M6ICAgdHJ1ZSxcbiAgICByb3dHYXA6ICAgICAgIDIyLFxuICAgIHJlZ2lvbkd1dHRlcjogMzYsXG4gIH0pO1xufVxuXG5mdW5jdGlvbiBjb21wdXRlRGVzY3JpcHRpdmVMYXlvdXQoc3JjUmVnaW9ucywgc3JjTm9kZXMpIHtcbiAgY29uc3QgbWV0YSA9IHJlYWRNZXRhKCk7XG4gIGNvbnN0IHsgaGVpZ2h0VGFibGUgfSA9IGJ1aWxkU2l6ZVRhYmxlKG1ldGEua2luZFN0eWxlcywgXCJkZXNjcmlwdGl2ZUhcIiwgbnVsbCk7XG4gIHJldHVybiBjb21wdXRlQ29tcGFjdExheW91dChzcmNSZWdpb25zLCBzcmNOb2Rlcywge1xuICAgIC8vIERyb3Agbm9kZXMgZmxhZ2dlZCBtaW5pbWFsT25seSAoZS5nLiB0aGUgc3ludGhldGljIHAtb3NcbiAgICAvLyBwbGF0Zm9ybS1sYXllciBhbmNob3IgdGhhdCBleGlzdHMgb25seSBzbyB0aGUgbWluaW1hbC12aWV3IHNwaW5lXG4gICAgLy8gYXJyb3cgaGFzIGEgZ2VuZXJpYyBcInBsYXRmb3JtXCIgdGFyZ2V0KS4gSW4gZGVzY3JpcHRpdmUgKyBkZXRhaWxlZFxuICAgIC8vIHRoZSB0aHJlZSByZWFsIE9TIGNhcmRzIHRha2Ugb3Zlci5cbiAgICBub2RlRmlsdGVyOiAgIChuKSA9PiAhbi5taW5pbWFsT25seSxcbiAgICBoZWlnaHRUYWJsZSxcbiAgICBjZW50ZXJSb3dzOiAgIGZhbHNlLFxuICAgIHJvd0dhcDogICAgICAgMTQsXG4gICAgcmVnaW9uR3V0dGVyOiAzMixcbiAgfSk7XG59XG5cbi8vIDVweCBzdGFuZC1vZmYgc28gdGhlIHBhdGggZ2VvbWV0cnkgc3RvcHMgb3V0c2lkZSB0aGUgY2FyZCBib3JkZXIuIFRoZVxuLy8gYXJyb3doZWFkIG1hcmtlciAobWFya2VyV2lkdGggNywgcmVmWCA5KSB0aGVuIHNpdHMgaW4gdGhhdCBnYXAgd2l0aCBpdHNcbi8vIHRpcCBqdXN0IHRvdWNoaW5nIHRoZSBjYXJkIGVkZ2Ug4oCUIGxpbmUgc3Ryb2tlIG5ldmVyIGxhbmRzIG9uIHRvcCBvZiB0aGVcbi8vIDFweCBjYXJkIGJvcmRlciwgZWxpbWluYXRpbmcgdGhlIFwic2NyYXRjaCBvdmVyIHRoZSByZWN0YW5nbGVcIiBvdmVybGFwLlxuY29uc3QgU0lERV9HQVAgPSA1O1xuZnVuY3Rpb24gc2lkZVBvaW50KG5vZGUsIHNpZGUpIHtcbiAgc3dpdGNoIChzaWRlKSB7XG4gICAgY2FzZSBcImxlZnRcIjogICByZXR1cm4gW25vZGUueCAtIFNJREVfR0FQLCAgICAgICAgICAgICAgbm9kZS55ICsgbm9kZS5oIC8gMl07XG4gICAgY2FzZSBcInJpZ2h0XCI6ICByZXR1cm4gW25vZGUueCArIG5vZGUudyArIFNJREVfR0FQLCAgICAgbm9kZS55ICsgbm9kZS5oIC8gMl07XG4gICAgY2FzZSBcInRvcFwiOiAgICByZXR1cm4gW25vZGUueCArIG5vZGUudyAvIDIsICAgICAgICAgICAgbm9kZS55IC0gU0lERV9HQVBdO1xuICAgIGNhc2UgXCJib3R0b21cIjogcmV0dXJuIFtub2RlLnggKyBub2RlLncgLyAyLCAgICAgICAgICAgIG5vZGUueSArIG5vZGUuaCArIFNJREVfR0FQXTtcbiAgfVxufVxuXG5mdW5jdGlvbiBvcnRob2dvbmFsUGF0aChbeDEsIHkxXSwgW3gyLCB5Ml0sIGZyb21TaWRlLCB0b1NpZGUpIHtcbiAgLy8gVGhyZWUtc2VnbWVudCBNYW5oYXR0YW4gcm91dGUgd2l0aCByb3VuZGVkIGNvcm5lcnMuIFVzZWQgYnkgdGhlXG4gIC8vIG1pbmltYWwtbW9kZSBzeW50aGVzaXplZCBlZGdlcyB3aGVyZSBjbGVhbiBhcmNoaXRlY3R1cmFsIGxpbmVzIHJlYWRcbiAgLy8gYmV0dGVyIHRoYW4gYmV6aWVyIGN1cnZlcy5cbiAgaWYgKGZyb21TaWRlID09PSBcImJvdHRvbVwiICYmIHRvU2lkZSA9PT0gXCJ0b3BcIikge1xuICAgIGlmIChNYXRoLmFicyh4MSAtIHgyKSA8IDMpIHJldHVybiBgTSAke3gxfSAke3kxfSBMICR7eDJ9ICR7eTJ9YDtcbiAgICBjb25zdCBtaWRZID0gKHkxICsgeTIpIC8gMjtcbiAgICBjb25zdCBzZ24gID0geDIgPiB4MSA/IDEgOiAtMTtcbiAgICBjb25zdCBjciAgID0gTWF0aC5taW4oMTIsIE1hdGguYWJzKHgyIC0geDEpIC8gMiwgTWF0aC5hYnMobWlkWSAtIHkxKSwgTWF0aC5hYnMoeTIgLSBtaWRZKSk7XG4gICAgcmV0dXJuIGBNICR7eDF9ICR7eTF9XG4gICAgICAgICAgICBMICR7eDF9ICR7bWlkWSAtIGNyfVxuICAgICAgICAgICAgUSAke3gxfSAke21pZFl9LCAke3gxICsgY3IgKiBzZ259ICR7bWlkWX1cbiAgICAgICAgICAgIEwgJHt4MiAtIGNyICogc2dufSAke21pZFl9XG4gICAgICAgICAgICBRICR7eDJ9ICR7bWlkWX0sICR7eDJ9ICR7bWlkWSArIGNyfVxuICAgICAgICAgICAgTCAke3gyfSAke3kyfWA7XG4gIH1cbiAgcmV0dXJuIGBNICR7eDF9ICR7eTF9IEwgJHt4Mn0gJHt5Mn1gO1xufVxuXG5mdW5jdGlvbiBiZXppZXJQYXRoKGEsIGIsIGVkZ2UpIHtcbiAgaWYgKGVkZ2Uucm91dGUgPT09IFwib3J0aG9nb25hbFwiKSByZXR1cm4gb3J0aG9nb25hbFBhdGgoYSwgYiwgZWRnZS5mcm9tU2lkZSwgZWRnZS50b1NpZGUpO1xuICBjb25zdCBbeDEsIHkxXSA9IGE7XG4gIGNvbnN0IFt4MiwgeTJdID0gYjtcbiAgY29uc3QgaG9yaXogPSBlZGdlLmZyb21TaWRlID09PSBcImxlZnRcIiB8fCBlZGdlLmZyb21TaWRlID09PSBcInJpZ2h0XCIgfHxcbiAgICAgICAgICAgICAgICBlZGdlLnRvU2lkZSAgID09PSBcImxlZnRcIiB8fCBlZGdlLnRvU2lkZSAgID09PSBcInJpZ2h0XCI7XG4gIGlmIChlZGdlLmJ5cGFzcykge1xuICAgIC8vIFJpZ2h0LXNpZGUgYnlwYXNzOiBkcm9wIGRvd24gb3V0c2lkZSB0aGUgYmFuZHMgdGhlbiBhcmMgYmFjayBpbi5cbiAgICBjb25zdCByYWlsWCA9IE1hdGgubWF4KHgxLCB4MikgKyA4MDtcbiAgICByZXR1cm4gYE0gJHt4MX0gJHt5MX0gQyAke3JhaWxYfSAke3kxfSwgJHtyYWlsWH0gJHt5Mn0sICR7eDJ9ICR7eTJ9YDtcbiAgfVxuICBpZiAoZWRnZS5sb25nUmFpbCkge1xuICAgIC8vIExvbmcgb2ZmLXN0YWdlIHJhaWwgZWRnZTogc291cmNlIHNpZGUg4oaSIHNpZGUgcmFpbCDihpIgdGFyZ2V0LiBSaWdodC1hbmdsZVxuICAgIC8vIEwgcGF0aCB3aXRoIHNtYWxsIGNoYW1mZXJzIHNvIHRoZSBsaW5lIHJlYWRzIGFzIFwib3V0IG9mIGJhbmRcIiBwbHVtYmluZy5cbiAgICBjb25zdCByYWlsWCA9IGVkZ2UubG9uZ1JhaWwgPT09IFwibGVmdFwiID8gMzAgOiAod2luZG93LlFPTF9ESUFHUkFNLkNBTlZBUy53IC0gMzApO1xuICAgIGNvbnN0IGsgPSAxNDsgLy8gY2hhbWZlclxuICAgIGNvbnN0IGdvRG93biA9IHkyID4geTE7XG4gICAgY29uc3QgcmFpbEVudHJ5WTEgPSB5MSArIChnb0Rvd24gPyBrIDogLWspO1xuICAgIGNvbnN0IHJhaWxFbnRyeVkyID0geTIgKyAoZ29Eb3duID8gLWsgOiBrKTtcbiAgICByZXR1cm4gYE0gJHt4MX0gJHt5MX1cbiAgICAgICAgICAgIEwgJHtyYWlsWCArIChyYWlsWCA+IHgxID8gLWsgOiBrKX0gJHt5MX1cbiAgICAgICAgICAgIFEgJHtyYWlsWH0gJHt5MX0sICR7cmFpbFh9ICR7cmFpbEVudHJ5WTF9XG4gICAgICAgICAgICBMICR7cmFpbFh9ICR7cmFpbEVudHJ5WTJ9XG4gICAgICAgICAgICBRICR7cmFpbFh9ICR7eTJ9LCAke3JhaWxYICsgKHJhaWxYID4geDIgPyAtayA6IGspfSAke3kyfVxuICAgICAgICAgICAgTCAke3gyfSAke3kyfWA7XG4gIH1cbiAgaWYgKGVkZ2Uud3JhcCkge1xuICAgIC8vIFdyYXAtYXJvdW5kOiBmcm9tIGJvdHRvbSBvZiByaWdodC1zaWRlIG5vZGUsIGRvd24gdGhlbiBsZWZ0IHRoZW4gdXAgaW50byB0b3Agb2YgbGVmdC1zaWRlIG5vZGUuXG4gICAgY29uc3QgcmFpbFkgPSAoeTEgKyB5MikgLyAyICsgMjQ7XG4gICAgcmV0dXJuIGBNICR7eDF9ICR7eTF9IEwgJHt4MX0gJHtyYWlsWX0gTCAke3gyfSAke3JhaWxZfSBMICR7eDJ9ICR7eTJ9YDtcbiAgfVxuICBpZiAoZWRnZS5kcm9wWCAhPT0gdW5kZWZpbmVkKSB7XG4gICAgLy8gVmVydGljYWwgZHJvcCB0byBhIHNwZWNpZmljIFggdGhlbiBvdmVyLlxuICAgIHJldHVybiBgTSAke3gxfSAke3kxfSBDICR7eDF9ICR7eTJ9LCAke3gyfSAke3kxfSwgJHt4Mn0gJHt5Mn1gO1xuICB9XG4gIGlmIChlZGdlLnNpZGVMb29wUmV0dXJuKSB7XG4gICAgLy8gQm90dG9tLXRvLWJvdHRvbSBsb29wIGFyY2luZyBiZWxvdyBib3RoIG5vZGVzLlxuICAgIGNvbnN0IGFyY1kgPSBNYXRoLm1heCh5MSwgeTIpICsgMjg7XG4gICAgcmV0dXJuIGBNICR7eDF9ICR7eTF9IEMgJHt4MX0gJHthcmNZfSwgJHt4Mn0gJHthcmNZfSwgJHt4Mn0gJHt5Mn1gO1xuICB9XG4gIGlmIChob3Jpeikge1xuICAgIGNvbnN0IGR4ID0gTWF0aC5tYXgoNDAsIE1hdGguYWJzKHgyIC0geDEpICogMC4zNSk7XG4gICAgY29uc3QgYzF4ID0geDEgKyAoZWRnZS5mcm9tU2lkZSA9PT0gXCJyaWdodFwiID8gIGR4IDogLWR4KTtcbiAgICBjb25zdCBjMnggPSB4MiArIChlZGdlLnRvU2lkZSAgID09PSBcInJpZ2h0XCIgPyAgZHggOiAtZHgpO1xuICAgIHJldHVybiBgTSAke3gxfSAke3kxfSBDICR7YzF4fSAke3kxfSwgJHtjMnh9ICR7eTJ9LCAke3gyfSAke3kyfWA7XG4gIH1cbiAgY29uc3QgZHkgPSBNYXRoLm1heCgyMCwgTWF0aC5hYnMoeTIgLSB5MSkgKiAwLjQ1KTtcbiAgY29uc3QgYzF5ID0geTEgKyAoZWRnZS5mcm9tU2lkZSA9PT0gXCJib3R0b21cIiA/ICBkeSA6IC1keSk7XG4gIGNvbnN0IGMyeSA9IHkyICsgKGVkZ2UudG9TaWRlICAgPT09IFwiYm90dG9tXCIgPyAgZHkgOiAtZHkpO1xuICByZXR1cm4gYE0gJHt4MX0gJHt5MX0gQyAke3gxfSAke2MxeX0sICR7eDJ9ICR7YzJ5fSwgJHt4Mn0gJHt5Mn1gO1xufVxuXG4vLyDilIDilIDilIAgcHJpbWl0aXZlIHJlbmRlcmVycyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIBcblxuZnVuY3Rpb24gUmVnaW9uKHsgcmVnaW9uIH0pIHtcbiAgY29uc3QgYWNjZW50VmFyID0gcmVnaW9uLmFjY2VudCA/IGB2YXIoLS0ke3JlZ2lvbi5hY2NlbnR9KWAgOiBcInZhcigtLWluay0zKVwiO1xuICByZXR1cm4gKFxuICAgIDxkaXYgY2xhc3NOYW1lPVwicmVnaW9uXCJcbiAgICAgICAgIGRhdGEtcmVnaW9uLWlkPXtyZWdpb24uaWR9XG4gICAgICAgICBkYXRhLXJlZ2lvbi1vcmQ9e3JlZ2lvbi5vcmR9XG4gICAgICAgICBkYXRhLWJvdW5kYXJ5PXtyZWdpb24uYm91bmRhcnkgfHwgXCJcIn1cbiAgICAgICAgIGRhdGEtZW50cnk9e3JlZ2lvbi5lbnRyeSA/IFwidHJ1ZVwiIDogdW5kZWZpbmVkfVxuICAgICAgICAgc3R5bGU9e3tcbiAgICAgICAgICAgbGVmdDogcmVnaW9uLngsIHRvcDogcmVnaW9uLnksIHdpZHRoOiByZWdpb24udywgaGVpZ2h0OiByZWdpb24uaCxcbiAgICAgICAgICAgXCItLXJlZ2lvbi1hY2NlbnRcIjogYWNjZW50VmFyLFxuICAgICAgICAgfX0+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInJlZ2lvbi1sYWJlbFwiPlxuICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJyZWdpb24tb3JkXCI+e3JlZ2lvbi5vcmR9PC9zcGFuPlxuICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJyZWdpb24tdGl0bGVcIj57cmVnaW9uLnRpdGxlfTwvc3Bhbj5cbiAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicmVnaW9uLWNhcHRpb25cIj57cmVnaW9uLmNhcHRpb259PC9zcGFuPlxuICAgICAgICB7cmVnaW9uLmxpZmV0aW1lICYmIDxzcGFuIGNsYXNzTmFtZT1cInJlZ2lvbi1saWZldGltZVwiPlt7cmVnaW9uLmxpZmV0aW1lfV08L3NwYW4+fVxuICAgICAgPC9kaXY+XG4gICAgPC9kaXY+XG4gICk7XG59XG5cbmZ1bmN0aW9uIE5vZGUoeyBub2RlLCBhY2NlbnQsIGhpZ2hsaWdodGVkLCBkaW1tZWQsIHRyYWNlZCwgZXhwYW5kZWQsIG9uSG92ZXIsIG9uTGVhdmUsIG9uQ2xpY2sgfSkge1xuICBjb25zdCBjbHMgPSBbXG4gICAgXCJub2RlXCIsIGBraW5kLSR7bm9kZS5raW5kfWAsXG4gICAgaGlnaGxpZ2h0ZWQgPyBcImlzLWhvdmVyXCIgOiBcIlwiLFxuICAgIGRpbW1lZCA/IFwiaXMtZGltXCIgOiBcIlwiLFxuICAgIHRyYWNlZCA/IFwiaXMtdHJhY2VkXCIgOiBcIlwiLFxuICAgIGV4cGFuZGVkID8gXCJpcy1leHBhbmRlZFwiIDogXCJcIixcbiAgICBub2RlLnBsYXRmb3JtID8gYHBsYXRmb3JtLSR7bm9kZS5wbGF0Zm9ybX1gIDogXCJcIixcbiAgXS5qb2luKFwiIFwiKTtcbiAgY29uc3QgYWNjZW50VmFyID0gYWNjZW50ID8gYHZhcigtLSR7YWNjZW50fSlgIDogXCJ2YXIoLS1pbmstMylcIjtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT17Y2xzfVxuICAgICAgICAgZGF0YS1ub2RlLWlkPXtub2RlLmlkfVxuICAgICAgICAgZGF0YS1yZWdpb249e25vZGUucmVnaW9ufVxuICAgICAgICAgc3R5bGU9e3tcbiAgICAgICAgICAgbGVmdDogbm9kZS54LCB0b3A6IG5vZGUueSwgd2lkdGg6IG5vZGUudywgaGVpZ2h0OiBub2RlLmgsXG4gICAgICAgICAgIFwiLS1yZWdpb24tYWNjZW50XCI6IGFjY2VudFZhcixcbiAgICAgICAgIH19XG4gICAgICAgICBvbk1vdXNlRW50ZXI9eygpID0+IG9uSG92ZXIobm9kZS5pZCl9XG4gICAgICAgICBvbk1vdXNlTGVhdmU9e29uTGVhdmV9XG4gICAgICAgICBvbkNsaWNrPXsoKSA9PiBvbkNsaWNrICYmIG9uQ2xpY2sobm9kZS5pZCl9PlxuICAgICAgPE5vZGVCb2R5IG5vZGU9e25vZGV9IC8+XG4gICAgPC9kaXY+XG4gICk7XG59XG5cbmZ1bmN0aW9uIE5vZGVCb2R5KHsgbm9kZSB9KSB7XG4gIGlmIChub2RlLmtpbmQgPT09IFwicGxhdGZvcm1cIikge1xuICAgIHJldHVybiAoXG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRmb3JtLWNhcmRcIj5cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJwbGF0Zm9ybS1oZWFkZXJcIj5cbiAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJwbGF0Zm9ybS1nbHlwaFwiPntub2RlLnBsYXRmb3JtID09PSBcImxpbnV4XCIgPyBcIkxcIiA6IG5vZGUucGxhdGZvcm0gPT09IFwibWFjb3NcIiA/IFwiTVwiIDogXCJXXCJ9PC9zcGFuPlxuICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInBsYXRmb3JtLW5hbWVcIj57bm9kZS5sYWJlbH08L3NwYW4+XG4gICAgICAgIDwvZGl2PlxuICAgICAgICA8dWwgY2xhc3NOYW1lPVwicGxhdGZvcm0tYnVsbGV0c1wiPlxuICAgICAgICAgIHtub2RlLmJ1bGxldHMubWFwKChiLCBpKSA9PiA8bGkga2V5PXtpfT57Yn08L2xpPil9XG4gICAgICAgIDwvdWw+XG4gICAgICAgIHtub2RlLm5vdGUgJiYgPGRpdiBjbGFzc05hbWU9XCJwbGF0Zm9ybS1ub3RlXCI+e25vZGUubm90ZX08L2Rpdj59XG4gICAgICAgIHtub2RlLmNvZGUgJiYgPGRpdiBjbGFzc05hbWU9XCJub2RlLWNvZGVcIj57bm9kZS5jb2RlfTwvZGl2Pn1cbiAgICAgIDwvZGl2PlxuICAgICk7XG4gIH1cbiAgaWYgKG5vZGUua2luZCA9PT0gXCJleHRcIikge1xuICAgIHJldHVybiAoXG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cImV4dC1jYXJkXCI+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwiZXh0LXJvd1wiPlxuICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cImV4dC1sYWJlbFwiPntub2RlLmxhYmVsfTwvc3Bhbj5cbiAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJleHQtcGlkXCI+e25vZGUucGlkfTwvc3Bhbj5cbiAgICAgICAgPC9kaXY+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwiZXh0LXN1YlwiPntub2RlLnN1Yn08L2Rpdj5cbiAgICAgICAge25vZGUuaW50ZXJuYWxzICYmIChcbiAgICAgICAgICA8dWwgY2xhc3NOYW1lPVwiZXh0LWludGVybmFsc1wiPlxuICAgICAgICAgICAge25vZGUuaW50ZXJuYWxzLm1hcCgoaXQsIGkpID0+IDxsaSBrZXk9e2l9PntpdH08L2xpPil9XG4gICAgICAgICAgPC91bD5cbiAgICAgICAgKX1cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJleHQtbWV0YVwiPlxuICAgICAgICAgIDxzcGFuPnBsdWdpbi50b21sPC9zcGFuPlxuICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT17YGV4dC1kYWVtb24gJHtub2RlLmRhZW1vbiA/IFwib25cIiA6IFwib2ZmXCJ9YH0+XG4gICAgICAgICAgICB7bm9kZS5kYWVtb24gPyBcIuKWoyBkYWVtb25cIiA6IFwi4pahIGVwaGVtZXJhbFwifVxuICAgICAgICAgIDwvc3Bhbj5cbiAgICAgICAgPC9kaXY+XG4gICAgICA8L2Rpdj5cbiAgICApO1xuICB9XG4gIGlmIChub2RlLmtpbmQgPT09IFwiYXBpXCIpIHtcbiAgICByZXR1cm4gKFxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJhcGktY2FyZFwiPlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImFwaS1yb3dcIj5cbiAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJhcGktbGFiZWxcIj57bm9kZS5sYWJlbH08L3NwYW4+XG4gICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwiYXBpLXBvcnRcIj4xMjcuMC4wLjE6NDI3MDA8L3NwYW4+XG4gICAgICAgIDwvZGl2PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImFwaS1zdWJcIj57bm9kZS5zdWJ9PC9kaXY+XG4gICAgICAgIHtub2RlLmlwYyAmJiA8ZGl2IGNsYXNzTmFtZT1cImFwaS1pcGNcIj57bm9kZS5pcGN9PC9kaXY+fVxuICAgICAgICB7bm9kZS5jb2RlICYmIDxkaXYgY2xhc3NOYW1lPVwibm9kZS1jb2RlXCI+e25vZGUuY29kZX08L2Rpdj59XG4gICAgICA8L2Rpdj5cbiAgICApO1xuICB9XG4gIGlmIChub2RlLmtpbmQgPT09IFwic3RhdGVcIikge1xuICAgIHJldHVybiAoXG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInN0YXRlLWNhcmRcIj5cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJhcGktcm93XCI+XG4gICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwiYXBpLWxhYmVsXCI+e25vZGUubGFiZWx9PC9zcGFuPlxuICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInN0YXRlLWJhZGdlXCI+dW5peCDCtyByZWFkLW9ubHk8L3NwYW4+XG4gICAgICAgIDwvZGl2PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImFwaS1zdWJcIj57bm9kZS5zdWJ9PC9kaXY+XG4gICAgICAgIHtub2RlLmlwYyAmJiA8ZGl2IGNsYXNzTmFtZT1cImFwaS1pcGNcIj57bm9kZS5pcGN9PC9kaXY+fVxuICAgICAgICB7bm9kZS5jb2RlICYmIDxkaXYgY2xhc3NOYW1lPVwibm9kZS1jb2RlXCI+e25vZGUuY29kZX08L2Rpdj59XG4gICAgICA8L2Rpdj5cbiAgICApO1xuICB9XG4gIGlmIChub2RlLmtpbmQgPT09IFwicm91dGVyXCIpIHtcbiAgICByZXR1cm4gKFxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJyb3V0ZXItY2FyZFwiPlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImFwaS1yb3dcIj5cbiAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJhcGktbGFiZWxcIj57bm9kZS5sYWJlbH08L3NwYW4+XG4gICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicm91dGVyLWJhZGdlXCI+T1MgdGhyZWFkPC9zcGFuPlxuICAgICAgICA8L2Rpdj5cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJhcGktc3ViXCI+e25vZGUuc3VifTwvZGl2PlxuICAgICAgICB7bm9kZS5pcGMgJiYgPGRpdiBjbGFzc05hbWU9XCJhcGktaXBjXCI+e25vZGUuaXBjfTwvZGl2Pn1cbiAgICAgICAge25vZGUuY29kZSAmJiA8ZGl2IGNsYXNzTmFtZT1cIm5vZGUtY29kZVwiPntub2RlLmNvZGV9PC9kaXY+fVxuICAgICAgPC9kaXY+XG4gICAgKTtcbiAgfVxuICBpZiAobm9kZS5raW5kID09PSBcInBzLWZpbGVcIikge1xuICAgIHJldHVybiAoXG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInBzZmlsZS1jYXJkXCI+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwicHNmaWxlLWhlYWRcIj5cbiAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJwc2ZpbGUtbGFiZWxcIj57bm9kZS5sYWJlbH08L3NwYW4+XG4gICAgICAgICAge25vZGUuZW52ICYmIDxzcGFuIGNsYXNzTmFtZT1cInBzZmlsZS1lbnZcIj5lbnY6IHtub2RlLmVudn08L3NwYW4+fVxuICAgICAgICA8L2Rpdj5cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJwc2ZpbGUtcGF0aFwiPntub2RlLnBhdGh9PC9kaXY+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwicHNmaWxlLXJ3XCI+XG4gICAgICAgICAge25vZGUud3JpdGVzICYmIG5vZGUud3JpdGVzLmxlbmd0aCA+IDAgJiYgKFxuICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicHNmaWxlLXdcIiB0aXRsZT1cIndyaXR0ZW4gYnlcIj5cbiAgICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicnctZ2x5cGhcIj53PC9zcGFuPlxuICAgICAgICAgICAgICB7bm9kZS53cml0ZXMuam9pbihcIiDCtyBcIil9XG4gICAgICAgICAgICA8L3NwYW4+XG4gICAgICAgICAgKX1cbiAgICAgICAgICB7bm9kZS5yZWFkcyAmJiBub2RlLnJlYWRzLmxlbmd0aCA+IDAgJiYgKFxuICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicHNmaWxlLXJcIiB0aXRsZT1cInJlYWQgYnlcIj5cbiAgICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwicnctZ2x5cGhcIj5yPC9zcGFuPlxuICAgICAgICAgICAgICB7bm9kZS5yZWFkcy5qb2luKFwiIMK3IFwiKX1cbiAgICAgICAgICAgIDwvc3Bhbj5cbiAgICAgICAgICApfVxuICAgICAgICA8L2Rpdj5cbiAgICAgIDwvZGl2PlxuICAgICk7XG4gIH1cbiAgaWYgKG5vZGUua2luZCA9PT0gXCJzdG9yZVwiKSB7XG4gICAgcmV0dXJuIChcbiAgICAgIDxkaXYgY2xhc3NOYW1lPXtgc3RvcmUtY2FyZCAke25vZGUuZXBoZW1lcmFsID8gXCJpcy1lcGhlbWVyYWxcIiA6IFwiXCJ9YH0+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwic3RvcmUtcm93XCI+XG4gICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwiZ2VuZXJpYy1sYWJlbFwiPntub2RlLmxhYmVsfTwvc3Bhbj5cbiAgICAgICAgICB7bm9kZS5lcGhlbWVyYWwgJiYgPHNwYW4gY2xhc3NOYW1lPVwic3RvcmUtYmFkZ2VcIj5lcGhlbWVyYWw8L3NwYW4+fVxuICAgICAgICA8L2Rpdj5cbiAgICAgICAge25vZGUuc3ViICYmIDxkaXYgY2xhc3NOYW1lPVwiZ2VuZXJpYy1zdWJcIj57bm9kZS5zdWJ9PC9kaXY+fVxuICAgICAgICB7bm9kZS5wYXRoICYmIDxkaXYgY2xhc3NOYW1lPVwic3RvcmUtcGF0aFwiPntub2RlLnBhdGh9PC9kaXY+fVxuICAgICAgPC9kaXY+XG4gICAgKTtcbiAgfVxuICAvLyBjb3JlIC8gZ2VuZXJpYyDigJQgc3VwcG9ydHMgYW4gb3B0aW9uYWwgYnVsbGV0cyBsaXN0IChlLmcuIGV2ZW50IGJ1cyB2YXJpYW50cyksXG4gIC8vIGEgYG5vdGVgIGZpZWxkIChzbWFsbCBpdGFsaWMgZm9vdGVyKSwgYW5kIHRoZSBgcGx1Zy1hbmNob3JgIHZhcmlhbnQgZm9yIGZhbi1pbiBub2Rlcy5cbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT1cImdlbmVyaWNcIj5cbiAgICAgIDxkaXYgY2xhc3NOYW1lPVwiZ2VuZXJpYy1sYWJlbFwiPntub2RlLmxhYmVsfTwvZGl2PlxuICAgICAge25vZGUuc3ViICYmIDxkaXYgY2xhc3NOYW1lPVwiZ2VuZXJpYy1zdWJcIj57bm9kZS5zdWJ9PC9kaXY+fVxuICAgICAge25vZGUuYnVsbGV0cyAmJiAoXG4gICAgICAgIDx1bCBjbGFzc05hbWU9XCJnZW5lcmljLWJ1bGxldHNcIj5cbiAgICAgICAgICB7bm9kZS5idWxsZXRzLm1hcCgoYiwgaSkgPT4gPGxpIGtleT17aX0+e2J9PC9saT4pfVxuICAgICAgICA8L3VsPlxuICAgICAgKX1cbiAgICAgIHtub2RlLm5vdGUgJiYgPGRpdiBjbGFzc05hbWU9XCJnZW5lcmljLW5vdGVcIj57bm9kZS5ub3RlfTwvZGl2Pn1cbiAgICAgIHtub2RlLmNvZGUgJiYgPGRpdiBjbGFzc05hbWU9XCJub2RlLWNvZGVcIj57bm9kZS5jb2RlfTwvZGl2Pn1cbiAgICA8L2Rpdj5cbiAgKTtcbn1cblxuLy8g4pSA4pSA4pSAIGJhbmQgZ3V0dGVyIGNoZXZyb25zIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgFxuXG5mdW5jdGlvbiBHdXR0ZXJzKCkge1xuICBjb25zdCB7IEdVVFRFUlMsIENBTlZBUyB9ID0gd2luZG93LlFPTF9ESUFHUkFNO1xuICBjb25zdCBjeCA9IENBTlZBUy53IC8gMjtcbiAgcmV0dXJuIChcbiAgICA8c3ZnIGNsYXNzTmFtZT1cImd1dHRlcnNcIiB2aWV3Qm94PXtgMCAwICR7Q0FOVkFTLnd9ICR7Q0FOVkFTLmh9YH0gcHJlc2VydmVBc3BlY3RSYXRpbz1cIm5vbmVcIj5cbiAgICAgIHtHVVRURVJTLm1hcCgoZywgaSkgPT4ge1xuICAgICAgICBjb25zdCBtaWQgPSAoZy5mcm9tWSArIGcudG9ZKSAvIDI7XG4gICAgICAgIGNvbnN0IHRvcCA9IGcuZnJvbVkgKyA0O1xuICAgICAgICBjb25zdCBib3QgPSBnLnRvWSAgIC0gNDtcbiAgICAgICAgY29uc3QgY2xzID0gYGd1dHRlciAke2cuZGFzaGVkID8gXCJpcy1kYXNoZWRcIiA6IFwiXCJ9IHRvbmUtJHtnLnRvbmUgfHwgXCJpbmtcIn1gO1xuICAgICAgICByZXR1cm4gKFxuICAgICAgICAgIDxnIGtleT17aX0gY2xhc3NOYW1lPXtjbHN9PlxuICAgICAgICAgICAgPGxpbmUgeDE9e2N4fSB5MT17dG9wfSAgICAgeDI9e2N4fSB5Mj17Ym90IC0gOH0gY2xhc3NOYW1lPVwiZ3V0dGVyLWxpbmVcIiAvPlxuICAgICAgICAgICAgey8qIGNoZXZyb24gKi99XG4gICAgICAgICAgICA8cGF0aCBkPXtgTSAke2N4IC0gNn0gJHtib3QgLSAxMH0gTCAke2N4fSAke2JvdH0gTCAke2N4ICsgNn0gJHtib3QgLSAxMH1gfSBjbGFzc05hbWU9XCJndXR0ZXItY2hldlwiIC8+XG4gICAgICAgICAgICB7LyogbGFiZWwgKi99XG4gICAgICAgICAgICA8dGV4dCB4PXtjeCArIDE0fSB5PXttaWQgKyAzfSBjbGFzc05hbWU9XCJndXR0ZXItbGFiZWxcIj57Zy5sYWJlbH08L3RleHQ+XG4gICAgICAgICAgPC9nPlxuICAgICAgICApO1xuICAgICAgfSl9XG4gICAgPC9zdmc+XG4gICk7XG59XG5cbi8vIOKUgOKUgOKUgCBlZGdlcyBsYXllciDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIBcblxuZnVuY3Rpb24gRWRnZXMoeyBub2RlcywgZWRnZXMsIHRyYWNlZFBhaXJzLCBjYW52YXNIIH0pIHtcbiAgY29uc3QgeyBDQU5WQVMgfSA9IHdpbmRvdy5RT0xfRElBR1JBTTtcbiAgY29uc3QgaCA9IGNhbnZhc0ggIT0gbnVsbCA/IGNhbnZhc0ggOiBDQU5WQVMuaDtcbiAgY29uc3Qgbm9kZU1hcCA9IHVzZU1lbW8oKCkgPT4gbmV3IE1hcChub2Rlcy5tYXAoKG4pID0+IFtuLmlkLCBuXSkpLCBbbm9kZXNdKTtcbiAgY29uc3Qgc3RhdGljUGF0aHMgPSB1c2VNZW1vKCgpID0+IHtcbiAgICByZXR1cm4gZWRnZXMubWFwKChlZGdlLCBpKSA9PiB7XG4gICAgICBjb25zdCBmcm9tID0gbm9kZU1hcC5nZXQoZWRnZS5mcm9tKTtcbiAgICAgIGNvbnN0IHRvICAgPSBub2RlTWFwLmdldChlZGdlLnRvKTtcbiAgICAgIGlmICghZnJvbSB8fCAhdG8pIHJldHVybiBudWxsO1xuICAgICAgY29uc3QgW2F1dG9Gcm9tLCBhdXRvVG9dID0gYXV0b1NpZGVzKGZyb20sIHRvKTtcbiAgICAgIGNvbnN0IGZyb21TaWRlID0gZWRnZS5mcm9tU2lkZSB8fCBhdXRvRnJvbTtcbiAgICAgIGNvbnN0IHRvU2lkZSAgID0gZWRnZS50b1NpZGUgICB8fCBhdXRvVG87XG4gICAgICBjb25zdCBhID0gc2lkZVBvaW50KGZyb20sIGZyb21TaWRlKTtcbiAgICAgIGNvbnN0IGIgPSBzaWRlUG9pbnQodG8sICAgdG9TaWRlKTtcbiAgICAgIHJldHVybiB7IGksIGVkZ2U6IHsgLi4uZWRnZSwgZnJvbVNpZGUsIHRvU2lkZSB9LCBkOiBiZXppZXJQYXRoKGEsIGIsIHsgLi4uZWRnZSwgZnJvbVNpZGUsIHRvU2lkZSB9KSB9O1xuICAgIH0pLmZpbHRlcihCb29sZWFuKTtcbiAgfSwgW2VkZ2VzLCBub2RlTWFwXSk7XG5cbiAgLy8gQ29tcHV0ZSB0cmFjZSBvdmVybGF5IHBhdGhzIChidWlsdCBmcm9tIGFjdGl2ZSB0cmFjZTsgbm90IGZyb20gRURHRVMpLlxuICBjb25zdCB0cmFjZVBhdGhzID0gdXNlTWVtbygoKSA9PiB7XG4gICAgaWYgKCF0cmFjZWRQYWlycykgcmV0dXJuIFtdO1xuICAgIHJldHVybiB0cmFjZWRQYWlycy5wYXRocztcbiAgfSwgW3RyYWNlZFBhaXJzXSk7XG5cbiAgcmV0dXJuIChcbiAgICA8c3ZnIGNsYXNzTmFtZT1cImVkZ2VzXCIgdmlld0JveD17YDAgMCAke0NBTlZBUy53fSAke2h9YH0gcHJlc2VydmVBc3BlY3RSYXRpbz1cIm5vbmVcIj5cbiAgICAgIDxkZWZzPlxuICAgICAgICA8bWFya2VyIGlkPVwiYXJyb3ctaW5rXCIgICB2aWV3Qm94PVwiMCAwIDEwIDEwXCIgcmVmWD1cIjlcIiByZWZZPVwiNVwiIG1hcmtlcldpZHRoPVwiN1wiIG1hcmtlckhlaWdodD1cIjdcIiBvcmllbnQ9XCJhdXRvLXN0YXJ0LXJldmVyc2VcIj5cbiAgICAgICAgICA8cGF0aCBkPVwiTSAwIDEgTCA5IDUgTCAwIDkgelwiIGNsYXNzTmFtZT1cImFycm93LWlua1wiIC8+XG4gICAgICAgIDwvbWFya2VyPlxuICAgICAgICA8bWFya2VyIGlkPVwiYXJyb3ctYW1iZXJcIiB2aWV3Qm94PVwiMCAwIDEwIDEwXCIgcmVmWD1cIjlcIiByZWZZPVwiNVwiIG1hcmtlcldpZHRoPVwiN1wiIG1hcmtlckhlaWdodD1cIjdcIiBvcmllbnQ9XCJhdXRvLXN0YXJ0LXJldmVyc2VcIj5cbiAgICAgICAgICA8cGF0aCBkPVwiTSAwIDEgTCA5IDUgTCAwIDkgelwiIGNsYXNzTmFtZT1cImFycm93LWFtYmVyXCIgLz5cbiAgICAgICAgPC9tYXJrZXI+XG4gICAgICAgIDxtYXJrZXIgaWQ9XCJhcnJvdy1zbGF0ZVwiIHZpZXdCb3g9XCIwIDAgMTAgMTBcIiByZWZYPVwiOVwiIHJlZlk9XCI1XCIgbWFya2VyV2lkdGg9XCI3XCIgbWFya2VySGVpZ2h0PVwiN1wiIG9yaWVudD1cImF1dG8tc3RhcnQtcmV2ZXJzZVwiPlxuICAgICAgICAgIDxwYXRoIGQ9XCJNIDAgMSBMIDkgNSBMIDAgOSB6XCIgY2xhc3NOYW1lPVwiYXJyb3ctc2xhdGVcIiAvPlxuICAgICAgICA8L21hcmtlcj5cbiAgICAgIDwvZGVmcz5cblxuICAgICAge3N0YXRpY1BhdGhzLm1hcCgoeyBpLCBlZGdlLCBkIH0pID0+IHtcbiAgICAgICAgY29uc3QgY2xzID0gW1xuICAgICAgICAgIFwiZWRnZVwiLCBgdG9uZS0ke2VkZ2UudG9uZSB8fCBcImlua1wifWAsXG4gICAgICAgICAgZWRnZS5kYXNoZWQgPyBcImlzLWRhc2hlZFwiIDogXCJcIixcbiAgICAgICAgICBlZGdlLmhhaXJsaW5lID8gXCJpcy1oYWlybGluZVwiIDogXCJcIixcbiAgICAgICAgICBlZGdlLmludGVybmFsID8gXCJpcy1pbnRlcm5hbFwiIDogXCJcIixcbiAgICAgICAgICBlZGdlLmJ5cGFzcyA/IFwiaXMtYnlwYXNzXCIgOiBcIlwiLFxuICAgICAgICAgIGVkZ2UubG9uZ1JhaWwgPyBcImlzLWxvbmdyYWlsXCIgOiBcIlwiLFxuICAgICAgICAgIHRyYWNlZFBhaXJzID8gXCJpcy1mYWRlXCIgOiBcIlwiLFxuICAgICAgICBdLmpvaW4oXCIgXCIpO1xuICAgICAgICAvLyBFdmVyeSBlZGdlIGdldHMgYW4gYXJyb3doZWFkIOKAlCBmbG93IGRpcmVjdGlvbiBoYXMgdG8gYmUgcmVhZGFibGVcbiAgICAgICAgLy8gd2l0aG91dCBjb250ZXh0IChUdWZ0ZTogbGluZXMgYXJlIG11bHRpdm9jYWw7IGV4cGxpY2l0IHRlcm1pbmF0b3JzXG4gICAgICAgIC8vIHBpbiBkb3duIHRoZSBzZW1hbnRpYykuIE9yaWdpbmFsbHkgb25seSBpbnRlcm5hbC9ieXBhc3MvbG9uZ1JhaWxcbiAgICAgICAgLy8gZWRnZXMgc2hvd2VkIGFycm93czsgY3Jvc3MtcmVnaW9uIGVkZ2VzIChzcGluZSkgd2VyZSBsZWZ0IHdpdGhvdXQsXG4gICAgICAgIC8vIHdoaWNoIG1hZGUgbWluaW1hbCB2aWV3IHJlYWQgYXMgYSBwYXNzaXZlIHdlYiByYXRoZXIgdGhhbiBhIGZsb3cuXG4gICAgICAgIGNvbnN0IHNob3dBcnJvdyA9IHRydWU7XG4gICAgICAgIHJldHVybiAoXG4gICAgICAgICAgPHBhdGgga2V5PXtgcy0ke2l9YH0gZD17ZH0gY2xhc3NOYW1lPXtjbHN9XG4gICAgICAgICAgICAgICAgbWFya2VyRW5kPXtzaG93QXJyb3cgPyBgdXJsKCNhcnJvdy0ke2VkZ2UudG9uZSB8fCBcImlua1wifSlgIDogXCJcIn0gLz5cbiAgICAgICAgKTtcbiAgICAgIH0pfVxuXG4gICAgICB7dHJhY2VQYXRocy5tYXAoKGQsIGkpID0+IChcbiAgICAgICAgPHBhdGgga2V5PXtgdC0ke2l9YH0gZD17ZH0gY2xhc3NOYW1lPXtgZWRnZSB0cmFjZS1lZGdlIHRvbmUtJHt0cmFjZWRQYWlycy50b25lfWB9XG4gICAgICAgICAgICAgIG1hcmtlckVuZD17YHVybCgjYXJyb3ctJHt0cmFjZWRQYWlycy50b25lfSlgfSAvPlxuICAgICAgKSl9XG4gICAgPC9zdmc+XG4gICk7XG59XG5cbi8vIOKUgOKUgOKUgCBtYWluIGRpYWdyYW0g4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSAXG5cbmZ1bmN0aW9uIERpYWdyYW0oeyB0d2Vha3MsIHNldFR3ZWFrIH0pIHtcbiAgY29uc3QgeyBSRUdJT05TOiBTUkNfUkVHSU9OUywgTk9ERVM6IFNSQ19OT0RFUywgVFJBQ0VTLCBDQU5WQVMgfSA9IHdpbmRvdy5RT0xfRElBR1JBTTtcbiAgY29uc3QgbGV2ZWwgICA9IHR3ZWFrcy5sZXZlbCB8fCBcIm1pbmltYWxcIjtcbiAgY29uc3QgY29tcGFjdCA9IGxldmVsID09PSBcIm1pbmltYWxcIjtcblxuICBjb25zdCBbaG92ZXJJZCwgc2V0SG92ZXJJZF0gPSB1c2VTdGF0ZShudWxsKTtcbiAgY29uc3QgW2V4cGFuZGVkSWRzLCBzZXRFeHBhbmRlZElkc10gPSB1c2VTdGF0ZSgoKSA9PiBuZXcgU2V0KCkpO1xuICBjb25zdCBbYWN0aXZlVHJhY2UsIHNldEFjdGl2ZVRyYWNlXSA9IHVzZVN0YXRlKG51bGwpO1xuICBjb25zdCBbc2NhbGUsIHNldFNjYWxlXSA9IHVzZVN0YXRlKDEpO1xuICBjb25zdCBvdXRlclJlZiA9IHVzZVJlZihudWxsKTtcblxuICBjb25zdCB0b2dnbGVFeHBhbmQgPSB1c2VDYWxsYmFjaygoaWQpID0+IHtcbiAgICBzZXRFeHBhbmRlZElkcygocHJldikgPT4ge1xuICAgICAgY29uc3QgbmV4dCA9IG5ldyBTZXQocHJldik7XG4gICAgICBpZiAobmV4dC5oYXMoaWQpKSBuZXh0LmRlbGV0ZShpZCk7XG4gICAgICBlbHNlIG5leHQuYWRkKGlkKTtcbiAgICAgIHJldHVybiBuZXh0O1xuICAgIH0pO1xuICB9LCBbXSk7XG5cbiAgLy8gbWluaW1hbCAgICAgIC0gdGllci0xIG5vZGVzIG9ubHksIHJvdy1wYWNrZWQsIGNlbnRlcmVkLlxuICAvLyBkZXNjcmlwdGl2ZSAgLSBhbGwgbm9kZXMsIHJvdy1wYWNrZWQgaW4gb3JpZ2luYWwgY29sdW1ucywgc2hvcnRlciBjYXJkcy5cbiAgLy8gZGV0YWlsZWQgICAgIC0gdGhlIHNvdXJjZSBsYXlvdXQgZnJvbSBkYXRhLmpzLCBhbGwgY2FyZHMgZnVsbHkgcmVuZGVyZWQuXG4gIGNvbnN0IGxheW91dCA9IHVzZU1lbW8oKCkgPT4ge1xuICAgIGlmIChsZXZlbCA9PT0gXCJtaW5pbWFsXCIpICAgICByZXR1cm4gY29tcHV0ZU1pbmltYWxMYXlvdXQoU1JDX1JFR0lPTlMsIFNSQ19OT0RFUyk7XG4gICAgaWYgKGxldmVsID09PSBcImRlc2NyaXB0aXZlXCIpIHJldHVybiBjb21wdXRlRGVzY3JpcHRpdmVMYXlvdXQoU1JDX1JFR0lPTlMsIFNSQ19OT0RFUyk7XG4gICAgLy8gRGV0YWlsZWQ6IHNvdXJjZSBsYXlvdXQsIGJ1dCBzdHJpcCBtaW5pbWFsT25seSBzeW50aGV0aWNzLlxuICAgIHJldHVybiB7IHJlZ2lvbnM6IFNSQ19SRUdJT05TLCBub2RlczogU1JDX05PREVTLmZpbHRlcigobikgPT4gIW4ubWluaW1hbE9ubHkpLCBjYW52YXNIOiBDQU5WQVMuaCB9O1xuICB9LCBbbGV2ZWwsIFNSQ19SRUdJT05TLCBTUkNfTk9ERVMsIENBTlZBUy5oXSk7XG5cbiAgY29uc3QgUkVHSU9OUyAgPSBsYXlvdXQucmVnaW9ucztcbiAgY29uc3QgTk9ERVMgICAgPSBsYXlvdXQubm9kZXM7XG4gIGNvbnN0IENBTlZBU19IID0gbGF5b3V0LmNhbnZhc0g7XG5cbiAgLy8gTGF5b3V0LWF3YXJlIG5vZGUgbG9va3VwLiBCb3RoIHRyYWNlIG92ZXJsYXlzIGFuZCB0aGUgZWRnZXMgbGF5ZXIgbmVlZFxuICAvLyBwb3NpdGlvbnMgZnJvbSB0aGUgY3VycmVudCBsYXlvdXQgcGFzcywgbm90IHRoZSBzb3VyY2UgY29vcmRzLlxuICBjb25zdCBub2RlTWFwID0gdXNlTWVtbygoKSA9PiBuZXcgTWFwKE5PREVTLm1hcCgobikgPT4gW24uaWQsIG5dKSksIFtOT0RFU10pO1xuXG4gIC8vIE1pbmltYWwgcmVuZGVycyBzeW50aGVzaXplZCBpbnRlci1yZWdpb24gZmxvdyAoTUVUQS5taW5pbWFsRmxvdyk7XG4gIC8vIGRlc2NyaXB0aXZlIGFuZCBkZXRhaWxlZCByZW5kZXIgdGhlIGZ1bGwgZWRnZSBzZXQgZnJvbSBkYXRhLmpzLlxuICBjb25zdCBTUkNfRURHRVMgPSB3aW5kb3cuUU9MX0RJQUdSQU0uRURHRVM7XG4gIGNvbnN0IE1FVEEgPSB3aW5kb3cuUU9MX0RJQUdSQU0uTUVUQSB8fCB7fTtcbiAgY29uc3QgdmlzaWJsZUVkZ2VzID0gdXNlTWVtbygoKSA9PiB7XG4gICAgcmV0dXJuIGxldmVsID09PSBcIm1pbmltYWxcIiA/IChNRVRBLm1pbmltYWxGbG93IHx8IFtdKSA6IFNSQ19FREdFUztcbiAgfSwgW2xldmVsLCBTUkNfRURHRVMsIE1FVEEubWluaW1hbEZsb3ddKTtcblxuICAvLyBNYXAgcmVnaW9uIGlkIOKGkiBhY2NlbnQgdG9rZW4gbmFtZSBzbyBSZWdpb24vTm9kZS9GbG93Q2hldnJvbnMgY2FuIHdyaXRlXG4gIC8vIHRoZSAtLXJlZ2lvbi1hY2NlbnQgQ1NTIGN1c3RvbSBwcm9wZXJ0eSBpbmxpbmUuIE1lYW5zIHRoZSBzdHlsZXNoZWV0XG4gIC8vIG5ldmVyIGhhcyB0byBrbm93IHdoaWNoIHJlZ2lvbiB1c2VzIHdoaWNoIGFjY2VudC5cbiAgY29uc3QgYWNjZW50QnlSZWdpb24gPSB1c2VNZW1vKCgpID0+IHtcbiAgICBjb25zdCBtID0gbmV3IE1hcCgpO1xuICAgIGZvciAoY29uc3QgciBvZiBSRUdJT05TKSBtLnNldChyLmlkLCByLmFjY2VudCB8fCBcImluay0zXCIpO1xuICAgIHJldHVybiBtO1xuICB9LCBbUkVHSU9OU10pO1xuXG4gIC8vIFN3aXRjaGluZyBiYWNrIHRvIG1pbmltYWwgZHJvcHMgdGhlIGNvbXBhY3QtaW5jb21wYXRpYmxlIHRyYWNlIG92ZXJsYXksXG4gIC8vIHNpbmNlIHRoZSBjb21wYWN0IGxheW91dCBvbWl0cyBhbnkgbm9uLXRpZXItMSB0cmFjZSBzdGVwLlxuICB1c2VFZmZlY3QoKCkgPT4ge1xuICAgIGlmIChjb21wYWN0ICYmIGFjdGl2ZVRyYWNlKSBzZXRBY3RpdmVUcmFjZShudWxsKTtcbiAgfSwgW2NvbXBhY3QsIGFjdGl2ZVRyYWNlXSk7XG5cbiAgLy8gQ2FyZCBwb3NpdGlvbnMgYW5kIHNpemVzIGNoYW5nZSBiZXR3ZWVuIGxheW91dHM7IGNvbGxhcHNlIGFueSBpbmxpbmVcbiAgLy8gZXhwYW5zaW9ucyB3aGVuIHRoZSBsZXZlbCBzd2l0Y2hlcyBzbyBhbiBleHBhbmRlZCBjYXJkIGZyb20gZGVzY3JpcHRpdmVcbiAgLy8gZG9lcyBub3QgYXdrd2FyZGx5IHBlcnNpc3QgaW50byBtaW5pbWFsIHdoZXJlIGl0cyBhYnNvbHV0ZSBzbG90IGlzIGdvbmUuXG4gIHVzZUVmZmVjdCgoKSA9PiB7XG4gICAgc2V0RXhwYW5kZWRJZHMobmV3IFNldCgpKTtcbiAgfSwgW2xldmVsXSk7XG5cbiAgLy8gRml0LXRvLXdpZHRoIHNjYWxpbmcgKGNhcHBlZCBhdCAxw5cpLiBTdGFnZSBzY3JvbGxzIHZlcnRpY2FsbHkuXG4gIHVzZUVmZmVjdCgoKSA9PiB7XG4gICAgZnVuY3Rpb24gZml0KCkge1xuICAgICAgY29uc3QgdG9wYmFyID0gZG9jdW1lbnQucXVlcnlTZWxlY3RvcihcIi50b3BiYXJcIik7XG4gICAgICBjb25zdCB0b3BIID0gdG9wYmFyID8gdG9wYmFyLmdldEJvdW5kaW5nQ2xpZW50UmVjdCgpLmhlaWdodCA6IDU2O1xuICAgICAgY29uc3QgcGFkID0gMzI7XG4gICAgICBjb25zdCB2dyAgPSB3aW5kb3cuaW5uZXJXaWR0aCAtIHBhZCAqIDI7XG4gICAgICBjb25zdCB2aCAgPSB3aW5kb3cuaW5uZXJIZWlnaHQgLSB0b3BIIC0gcGFkICogMjtcbiAgICAgIGxldCBzID0gdncgLyBDQU5WQVMudztcbiAgICAgIHMgPSBNYXRoLm1pbihzLCAxLjApO1xuICAgICAgY29uc3QgZml0c0hlaWdodCA9IENBTlZBU19IICogcyA8PSB2aDtcbiAgICAgIGlmIChmaXRzSGVpZ2h0KSB7XG4gICAgICAgIHMgPSBNYXRoLm1pbih2aCAvIENBTlZBU19ILCAxLjAsIHZ3IC8gQ0FOVkFTLncpO1xuICAgICAgfVxuICAgICAgc2V0U2NhbGUoTWF0aC5tYXgoMC4zNSwgcykpO1xuICAgIH1cbiAgICBmaXQoKTtcbiAgICB3aW5kb3cuYWRkRXZlbnRMaXN0ZW5lcihcInJlc2l6ZVwiLCBmaXQpO1xuICAgIGNvbnN0IHQgPSBzZXRUaW1lb3V0KGZpdCwgMzAwKTtcbiAgICByZXR1cm4gKCkgPT4geyB3aW5kb3cucmVtb3ZlRXZlbnRMaXN0ZW5lcihcInJlc2l6ZVwiLCBmaXQpOyBjbGVhclRpbWVvdXQodCk7IH07XG4gIH0sIFtDQU5WQVMudywgQ0FOVkFTX0hdKTtcblxuICAvLyBUcmFjZSBzdGF0ZSBkZXJpdmF0aW9ucy5cbiAgY29uc3QgdHJhY2VkTm9kZUlkcyA9IHVzZU1lbW8oKCkgPT4gYWN0aXZlVHJhY2UgPyBuZXcgU2V0KGFjdGl2ZVRyYWNlLnN0ZXBzKSA6IG51bGwsIFthY3RpdmVUcmFjZV0pO1xuICBjb25zdCB0cmFjZWRQYWlycyAgID0gdXNlTWVtbygoKSA9PiB7XG4gICAgaWYgKCFhY3RpdmVUcmFjZSkgcmV0dXJuIG51bGw7XG4gICAgY29uc3QgcGF0aHMgPSBbXTtcbiAgICBmb3IgKGxldCBpID0gMDsgaSA8IGFjdGl2ZVRyYWNlLnN0ZXBzLmxlbmd0aCAtIDE7IGkrKykge1xuICAgICAgY29uc3QgYSA9IG5vZGVNYXAuZ2V0KGFjdGl2ZVRyYWNlLnN0ZXBzW2ldKTtcbiAgICAgIGNvbnN0IGIgPSBub2RlTWFwLmdldChhY3RpdmVUcmFjZS5zdGVwc1tpICsgMV0pO1xuICAgICAgaWYgKCFhIHx8ICFiKSBjb250aW51ZTtcbiAgICAgIGNvbnN0IFtmcm9tU2lkZSwgdG9TaWRlXSA9IGF1dG9TaWRlcyhhLCBiKTtcbiAgICAgIGNvbnN0IHBhID0gc2lkZVBvaW50KGEsIGZyb21TaWRlKTtcbiAgICAgIGNvbnN0IHBiID0gc2lkZVBvaW50KGIsIHRvU2lkZSk7XG4gICAgICBwYXRocy5wdXNoKGJlemllclBhdGgocGEsIHBiLCB7IGZyb21TaWRlLCB0b1NpZGUgfSkpO1xuICAgIH1cbiAgICByZXR1cm4geyBwYXRocywgdG9uZTogYWN0aXZlVHJhY2UudG9uZSB8fCBcImlua1wiIH07XG4gIH0sIFthY3RpdmVUcmFjZSwgbm9kZU1hcF0pO1xuXG4gIGNvbnN0IG9uU2VsZWN0VHJhY2UgPSAodCkgPT4ge1xuICAgIC8vIFRyYWNlIG5vZGVzIGJlbG9uZyB0byB0aGUgb3JpZ2luYWwgbGF5b3V0LCBub3QgdGhlIGNvbXBhY3Qgb25lLFxuICAgIC8vIHNvIHBpY2tpbmcgYSB0cmFjZSB3aGlsZSBpbiBtaW5pbWFsIGFsc28gZXhwYW5kcyB0aGUgZGlhZ3JhbS5cbiAgICBpZiAoY29tcGFjdCAmJiBzZXRUd2Vhaykgc2V0VHdlYWsoXCJsZXZlbFwiLCBcImRlc2NyaXB0aXZlXCIpO1xuICAgIHNldEFjdGl2ZVRyYWNlKChwcmV2KSA9PiAocHJldiAmJiBwcmV2LmlkID09PSB0LmlkID8gbnVsbCA6IHQpKTtcbiAgICBzZXRFeHBhbmRlZElkcyhuZXcgU2V0KCkpO1xuICB9O1xuXG4gIHJldHVybiAoXG4gICAgPGRpdiBjbGFzc05hbWU9XCJkaWFncmFtLXJvb3RcIj5cbiAgICAgIDxUb3BiYXJcbiAgICAgICAgYWN0aXZlVHJhY2U9e2FjdGl2ZVRyYWNlfSB0cmFjZXM9e1RSQUNFU30gb25TZWxlY3RUcmFjZT17b25TZWxlY3RUcmFjZX1cbiAgICAgICAgbGV2ZWw9e3R3ZWFrcy5sZXZlbCB8fCBcIm1pbmltYWxcIn1cbiAgICAgICAgc2V0TGV2ZWw9eyhsKSA9PiBzZXRUd2VhayAmJiBzZXRUd2VhayhcImxldmVsXCIsIGwpfVxuICAgICAgLz5cblxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJzdGFnZS1vdXRlclwiIHJlZj17b3V0ZXJSZWZ9PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInN0YWdlLXdyYXBcIiBzdHlsZT17eyB3aWR0aDogQ0FOVkFTLncgKiBzY2FsZSwgaGVpZ2h0OiBDQU5WQVNfSCAqIHNjYWxlIH19PlxuICAgICAgICAgIDxkaXYgY2xhc3NOYW1lPVwic3RhZ2VcIiBzdHlsZT17eyB3aWR0aDogQ0FOVkFTLncsIGhlaWdodDogQ0FOVkFTX0gsIHRyYW5zZm9ybTogYHNjYWxlKCR7c2NhbGV9KWAgfX0+XG4gICAgICAgICAgICA8UGFwZXJCYWNrZHJvcCAvPlxuXG4gICAgICAgICAgICB7UkVHSU9OUy5tYXAoKHIpID0+IDxSZWdpb24ga2V5PXtyLmlkfSByZWdpb249e3J9IC8+KX1cblxuICAgICAgICAgICAge2xldmVsICE9PSBcImRldGFpbGVkXCIgJiYgPEZsb3dDaGV2cm9ucyByZWdpb25zPXtSRUdJT05TfSBjYW52YXNXPXtDQU5WQVMud30gY2FudmFzSD17Q0FOVkFTX0h9IGFjY2VudEJ5UmVnaW9uPXthY2NlbnRCeVJlZ2lvbn0gLz59XG4gICAgICAgICAgICB7bGV2ZWwgPT09IFwiZGV0YWlsZWRcIiAmJiA8UXVhZHJhbnRzIC8+fVxuICAgICAgICAgICAge2xldmVsID09PSBcImRldGFpbGVkXCIgJiYgPEd1dHRlcnMgLz59XG4gICAgICAgICAgICB7bGV2ZWwgPT09IFwiZGV0YWlsZWRcIiAmJiA8VG9raW9Cb3VuZGFyeSAvPn1cbiAgICAgICAgICAgIHtsZXZlbCA9PT0gXCJkZXRhaWxlZFwiICYmIDxMYW5lTGFiZWxzIC8+fVxuICAgICAgICAgICAgPEVkZ2VzIG5vZGVzPXtOT0RFU30gZWRnZXM9e3Zpc2libGVFZGdlc30gdHJhY2VkUGFpcnM9e3RyYWNlZFBhaXJzfSBjYW52YXNIPXtDQU5WQVNfSH0gLz5cblxuICAgICAgICAgICAge05PREVTLm1hcCgobikgPT4ge1xuICAgICAgICAgICAgICBjb25zdCBkaW1tZWQgPSB0cmFjZWROb2RlSWRzICYmICF0cmFjZWROb2RlSWRzLmhhcyhuLmlkKTtcbiAgICAgICAgICAgICAgY29uc3QgdHJhY2VkID0gdHJhY2VkTm9kZUlkcyAmJiB0cmFjZWROb2RlSWRzLmhhcyhuLmlkKTtcbiAgICAgICAgICAgICAgY29uc3QgaGlnaGxpZ2h0ZWQgPSBob3ZlcklkID09PSBuLmlkO1xuICAgICAgICAgICAgICBjb25zdCBleHBhbmRlZCA9IGV4cGFuZGVkSWRzLmhhcyhuLmlkKTtcbiAgICAgICAgICAgICAgcmV0dXJuIChcbiAgICAgICAgICAgICAgICA8Tm9kZSBrZXk9e24uaWR9IG5vZGU9e259IGFjY2VudD17YWNjZW50QnlSZWdpb24uZ2V0KG4ucmVnaW9uKX1cbiAgICAgICAgICAgICAgICAgICAgICBoaWdobGlnaHRlZD17aGlnaGxpZ2h0ZWR9IGRpbW1lZD17ZGltbWVkfSB0cmFjZWQ9e3RyYWNlZH1cbiAgICAgICAgICAgICAgICAgICAgICBleHBhbmRlZD17ZXhwYW5kZWR9XG4gICAgICAgICAgICAgICAgICAgICAgb25Ib3Zlcj17c2V0SG92ZXJJZH0gb25MZWF2ZT17KCkgPT4gc2V0SG92ZXJJZChudWxsKX1cbiAgICAgICAgICAgICAgICAgICAgICBvbkNsaWNrPXt0b2dnbGVFeHBhbmR9IC8+XG4gICAgICAgICAgICAgICk7XG4gICAgICAgICAgICB9KX1cblxuICAgICAgICAgICAgPENvcm5lck1hcmtzIGNhbnZhc0g9e0NBTlZBU19IfSAvPlxuICAgICAgICAgICAgPFBsYXRlQW5ub3RhdGlvbnMgYWN0aXZlVHJhY2U9e2FjdGl2ZVRyYWNlfSBjYW52YXNIPXtDQU5WQVNfSH0gLz5cbiAgICAgICAgICA8L2Rpdj5cbiAgICAgICAgPC9kaXY+XG4gICAgICA8L2Rpdj5cblxuICAgICAgPERldGFpbFBhbmVsIGFjdGl2ZVRyYWNlPXthY3RpdmVUcmFjZX1cbiAgICAgICAgICAgICAgICAgICBvbkNsb3NlPXsoKSA9PiBzZXRBY3RpdmVUcmFjZShudWxsKX0gLz5cbiAgICA8L2Rpdj5cbiAgKTtcbn1cblxuLy8gRG93bndhcmQgY2hldnJvbnMgYmV0d2VlbiBjb25zZWN1dGl2ZSByZWdpb25zIC0gcmVhZHMgYXMgYSBudW1iZXJlZCBmbG93XG4vLyBwYXRoIHRocm91Z2ggcjEg4oaSIHIyIOKGkiAuLi4g4oaSIHI2LiBDb21wdXRlZCBmcm9tIHRoZSBjdXJyZW50IGxheW91dCBzbyBpdFxuLy8gZm9sbG93cyBtaW5pbWFsIGFuZCBkZXNjcmlwdGl2ZSBjb21wYWN0aW9uLCBub3QgdGhlIHNvdXJjZSBjb29yZHMuXG5mdW5jdGlvbiBGbG93Q2hldnJvbnMoeyByZWdpb25zLCBjYW52YXNXLCBjYW52YXNILCBhY2NlbnRCeVJlZ2lvbiB9KSB7XG4gIGlmIChyZWdpb25zLmxlbmd0aCA8IDIpIHJldHVybiBudWxsO1xuICBjb25zdCBjeCA9IGNhbnZhc1cgLyAyO1xuICByZXR1cm4gKFxuICAgIDxzdmcgY2xhc3NOYW1lPVwiZmxvdy1jaGV2cm9uc1wiIHZpZXdCb3g9e2AwIDAgJHtjYW52YXNXfSAke2NhbnZhc0h9YH0gcHJlc2VydmVBc3BlY3RSYXRpbz1cIm5vbmVcIj5cbiAgICAgIHtyZWdpb25zLnNsaWNlKDAsIC0xKS5tYXAoKHIsIGkpID0+IHtcbiAgICAgICAgY29uc3QgbmV4dCA9IHJlZ2lvbnNbaSArIDFdO1xuICAgICAgICBjb25zdCBjeSA9IChyLnkgKyByLmggKyBuZXh0LnkpIC8gMjtcbiAgICAgICAgY29uc3QgYWNjZW50ID0gYWNjZW50QnlSZWdpb24gJiYgYWNjZW50QnlSZWdpb24uZ2V0KHIuaWQpO1xuICAgICAgICBjb25zdCBzdHlsZSA9IGFjY2VudCA/IHsgXCItLXJlZ2lvbi1hY2NlbnRcIjogYHZhcigtLSR7YWNjZW50fSlgIH0gOiB1bmRlZmluZWQ7XG4gICAgICAgIHJldHVybiAoXG4gICAgICAgICAgPGcga2V5PXtpfSBjbGFzc05hbWU9XCJmbG93LWNoZXZcIiBzdHlsZT17c3R5bGV9PlxuICAgICAgICAgICAgPHBhdGggZD17YE0gJHtjeCAtIDE4fSAke2N5IC0gOH0gTCAke2N4fSAke2N5ICsgNn0gTCAke2N4ICsgMTh9ICR7Y3kgLSA4fWB9IC8+XG4gICAgICAgICAgPC9nPlxuICAgICAgICApO1xuICAgICAgfSl9XG4gICAgPC9zdmc+XG4gICk7XG59XG5cbmZ1bmN0aW9uIFF1YWRyYW50cygpIHtcbiAgY29uc3QgeyBNRVRBLCBDQU5WQVMgfSA9IHdpbmRvdy5RT0xfRElBR1JBTTtcbiAgaWYgKCFNRVRBIHx8ICFNRVRBLnF1YWRyYW50cykgcmV0dXJuIG51bGw7XG4gIC8vIENvbXB1dGUgdGhlIGNyb3NzLWRpdmlkZXIgZXh0ZW50cyBmcm9tIHRoZSBmb3VyIHF1YWRyYW50IGJvdW5kcy5cbiAgY29uc3QgeHMgPSBNRVRBLnF1YWRyYW50cy5tYXAoKHEpID0+IHEueCk7XG4gIGNvbnN0IHdzID0gTUVUQS5xdWFkcmFudHMubWFwKChxKSA9PiBxLnggKyBxLncpO1xuICBjb25zdCB5cyA9IE1FVEEucXVhZHJhbnRzLm1hcCgocSkgPT4gcS55KTtcbiAgY29uc3QgaHMgPSBNRVRBLnF1YWRyYW50cy5tYXAoKHEpID0+IHEueSArIHEuaCk7XG4gIGNvbnN0IGxlZnQgPSBNYXRoLm1pbiguLi54cyksIHJpZ2h0ID0gTWF0aC5tYXgoLi4ud3MpO1xuICBjb25zdCB0b3AgID0gTWF0aC5taW4oLi4ueXMpLCBib3QgICA9IE1hdGgubWF4KC4uLmhzKTtcbiAgLy8gVmVydGljYWwgZGl2aWRlciBiZXR3ZWVuIGxlZnQgJiByaWdodCBoYWx2ZXMuXG4gIGNvbnN0IG1pZFggPSAoTWF0aC5tYXgoLi4ueHMuZmlsdGVyKCh4LCBpKSA9PiBNRVRBLnF1YWRyYW50c1tpXS5pZC5lbmRzV2l0aChcImxcIikpKSArXG4gICAgICAgICAgICAgICAgTWF0aC5taW4oLi4ueHMuZmlsdGVyKCh4LCBpKSA9PiBNRVRBLnF1YWRyYW50c1tpXS5pZC5lbmRzV2l0aChcInJcIikpKSkgLyAyO1xuICAvLyBIb3Jpem9udGFsIGRpdmlkZXIgYmV0d2VlbiB0b3AgJiBib3R0b20gaGFsdmVzLlxuICBjb25zdCBtaWRZID0gKE1hdGgubWF4KC4uLnlzLmZpbHRlcigoeSwgaSkgPT4gTUVUQS5xdWFkcmFudHNbaV0uaWQuc3RhcnRzV2l0aChcInRcIikpKSArXG4gICAgICAgICAgICAgICAgTWF0aC5taW4oLi4ueXMuZmlsdGVyKCh5LCBpKSA9PiBNRVRBLnF1YWRyYW50c1tpXS5pZC5zdGFydHNXaXRoKFwiYlwiKSkpKSAvIDI7XG5cbiAgcmV0dXJuIChcbiAgICA8UmVhY3QuRnJhZ21lbnQ+XG4gICAgICB7TUVUQS5xdWFkcmFudHMubWFwKChxKSA9PiAoXG4gICAgICAgIDxkaXYga2V5PXtxLmlkfSBjbGFzc05hbWU9e2BxdWFkcmFudCBxdWFkcmFudC0ke3EuaWR9YH1cbiAgICAgICAgICAgICBzdHlsZT17eyBsZWZ0OiBxLngsIHRvcDogcS55LCB3aWR0aDogcS53LCBoZWlnaHQ6IHEuaCB9fT5cbiAgICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInF1YWRyYW50LWxhYmVsXCI+XG4gICAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJxdWFkcmFudC1nbHlwaFwiPntxLmdseXBofTwvc3Bhbj5cbiAgICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInF1YWRyYW50LW9yZFwiPntxLm9yZH08L3NwYW4+XG4gICAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJxdWFkcmFudC10aXRsZVwiPntxLnRpdGxlfTwvc3Bhbj5cbiAgICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInF1YWRyYW50LWF4ZXNcIj5cbiAgICAgICAgICAgICAgPHNwYW4+e3EuYXhpc1h9PC9zcGFuPlxuICAgICAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJheGVzLWRvdFwiPsK3PC9zcGFuPlxuICAgICAgICAgICAgICA8c3Bhbj57cS5heGlzWX08L3NwYW4+XG4gICAgICAgICAgICA8L3NwYW4+XG4gICAgICAgICAgPC9kaXY+XG4gICAgICAgIDwvZGl2PlxuICAgICAgKSl9XG4gICAgICA8c3ZnIGNsYXNzTmFtZT1cInF1YWRyYW50LWNyb3NzXCIgdmlld0JveD17YDAgMCAke0NBTlZBUy53fSAke0NBTlZBUy5ofWB9IHByZXNlcnZlQXNwZWN0UmF0aW89XCJub25lXCI+XG4gICAgICAgIDxsaW5lIHgxPXttaWRYfSB5MT17dG9wfSAgeDI9e21pZFh9IHkyPXtib3R9IGNsYXNzTmFtZT1cInFjcm9zcy1saW5lXCIgLz5cbiAgICAgICAgPGxpbmUgeDE9e2xlZnR9IHkxPXttaWRZfSB4Mj17cmlnaHR9IHkyPXttaWRZfSBjbGFzc05hbWU9XCJxY3Jvc3MtbGluZVwiIC8+XG4gICAgICA8L3N2Zz5cbiAgICA8L1JlYWN0LkZyYWdtZW50PlxuICApO1xufVxuXG5mdW5jdGlvbiBUb2tpb0JvdW5kYXJ5KCkge1xuICBjb25zdCB7IE1FVEEsIFJFR0lPTlMgfSA9IHdpbmRvdy5RT0xfRElBR1JBTTtcbiAgaWYgKCFNRVRBIHx8ICFNRVRBLnRva2lvQm91bmRhcnkpIHJldHVybiBudWxsO1xuICBjb25zdCByMyA9IFJFR0lPTlMuZmluZCgocikgPT4gci5pZCA9PT0gXCJyM1wiKTtcbiAgaWYgKCFyMykgcmV0dXJuIG51bGw7XG4gIGNvbnN0IHkgPSBNRVRBLnRva2lvQm91bmRhcnkueTtcbiAgY29uc3QgbGVmdCA9IHIzLnggKyAxMjtcbiAgY29uc3QgcmlnaHQgPSByMy54ICsgcjMudyAtIDEyO1xuICByZXR1cm4gKFxuICAgIDxzdmcgY2xhc3NOYW1lPVwidG9raW8tYm91bmRhcnlcIiB2aWV3Qm94PXtgMCAwICR7d2luZG93LlFPTF9ESUFHUkFNLkNBTlZBUy53fSAke3dpbmRvdy5RT0xfRElBR1JBTS5DQU5WQVMuaH1gfVxuICAgICAgICAgcHJlc2VydmVBc3BlY3RSYXRpbz1cIm5vbmVcIj5cbiAgICAgIHsvKiBUaGUgZGFzaGVkIGxpbmUgaXRzZWxmICovfVxuICAgICAgPGxpbmUgeDE9e2xlZnR9IHkxPXt5fSB4Mj17cmlnaHR9IHkyPXt5fSBjbGFzc05hbWU9XCJ0b2tpby1ydWxlXCIgLz5cbiAgICAgIHsvKiBFbmQgdGlja3MgKi99XG4gICAgICA8bGluZSB4MT17bGVmdH0gIHkxPXt5IC0gNn0geDI9e2xlZnR9ICB5Mj17eSArIDZ9IGNsYXNzTmFtZT1cInRva2lvLXRpY2tcIiAvPlxuICAgICAgPGxpbmUgeDE9e3JpZ2h0fSB5MT17eSAtIDZ9IHgyPXtyaWdodH0geTI9e3kgKyA2fSBjbGFzc05hbWU9XCJ0b2tpby10aWNrXCIgLz5cbiAgICAgIHsvKiBDZW50ZXJlZCBsYWJlbCAqL31cbiAgICAgIDxyZWN0IHg9eyhsZWZ0ICsgcmlnaHQpIC8gMiAtIDExMH0geT17eSAtIDExfSB3aWR0aD1cIjIyMFwiIGhlaWdodD1cIjIyXCIgY2xhc3NOYW1lPVwidG9raW8tbGFiZWwtYmdcIiAvPlxuICAgICAgPHRleHQgeD17KGxlZnQgKyByaWdodCkgLyAyfSB5PXt5ICsgNH0gY2xhc3NOYW1lPVwidG9raW8tbGFiZWwtdGV4dFwiPlxuICAgICAgICDihpMgIHRva2lvIG11bHRpLXRocmVhZCBydW50aW1lICDihpNcbiAgICAgIDwvdGV4dD5cbiAgICAgIHsvKiBQaGFzZSBsYWJlbHMgYXQgdGhlIGVuZHMgKi99XG4gICAgICA8dGV4dCB4PXtsZWZ0ICsgNn0geT17eSAtIDEyfSBjbGFzc05hbWU9XCJ0b2tpby1waGFzZS10ZXh0XCI+cHJlLXRva2lvIMK3IG1haW4gdGhyZWFkPC90ZXh0PlxuICAgICAgPHRleHQgeD17cmlnaHQgLSA2fSB5PXt5IC0gMTJ9IGNsYXNzTmFtZT1cInRva2lvLXBoYXNlLXRleHRcIiB0ZXh0QW5jaG9yPVwiZW5kXCI+YXN5bmMgdGFza3M8L3RleHQ+XG4gICAgPC9zdmc+XG4gICk7XG59XG5cbmZ1bmN0aW9uIExhbmVMYWJlbHMoKSB7XG4gIGNvbnN0IHsgTUVUQSwgQ0FOVkFTIH0gPSB3aW5kb3cuUU9MX0RJQUdSQU07XG4gIGlmICghTUVUQSB8fCAhTUVUQS5sYW5lTGFiZWxzKSByZXR1cm4gbnVsbDtcbiAgcmV0dXJuIChcbiAgICA8c3ZnIGNsYXNzTmFtZT1cImxhbmUtbGFiZWxzXCIgdmlld0JveD17YDAgMCAke0NBTlZBUy53fSAke0NBTlZBUy5ofWB9IHByZXNlcnZlQXNwZWN0UmF0aW89XCJub25lXCI+XG4gICAgICB7TUVUQS5sYW5lTGFiZWxzLm1hcCgobCwgaSkgPT4gKFxuICAgICAgICA8ZyBrZXk9e2l9IHRyYW5zZm9ybT17YHRyYW5zbGF0ZSgke2wueH0sICR7bC55ICsgbC5oIC8gMn0pYH0+XG4gICAgICAgICAgPHRleHQgY2xhc3NOYW1lPVwibGFuZS1sYWJlbC10ZXh0XCIgdHJhbnNmb3JtPVwicm90YXRlKC05MClcIj57bC5sYWJlbH08L3RleHQ+XG4gICAgICAgIDwvZz5cbiAgICAgICkpfVxuICAgIDwvc3ZnPlxuICApO1xufVxuXG5mdW5jdGlvbiBQYXBlckJhY2tkcm9wKCkgeyByZXR1cm4gPGRpdiBjbGFzc05hbWU9XCJwYXBlci1iYWNrZHJvcFwiIGFyaWEtaGlkZGVuPVwidHJ1ZVwiIC8+OyB9XG5cbmZ1bmN0aW9uIENvcm5lck1hcmtzKHsgY2FudmFzSCB9KSB7XG4gIGNvbnN0IHsgQ0FOVkFTIH0gPSB3aW5kb3cuUU9MX0RJQUdSQU07XG4gIGNvbnN0IGggPSBjYW52YXNIICE9IG51bGwgPyBjYW52YXNIIDogQ0FOVkFTLmg7XG4gIGNvbnN0IG1hcmsgPSAoeCwgeSwgZHgsIGR5KSA9PiAoXG4gICAgPGc+XG4gICAgICA8bGluZSB4MT17eH0geTE9e3l9IHgyPXt4ICsgZHh9IHkyPXt5fSBjbGFzc05hbWU9XCJtYXJrXCIgLz5cbiAgICAgIDxsaW5lIHgxPXt4fSB5MT17eX0geDI9e3h9IHkyPXt5ICsgZHl9IGNsYXNzTmFtZT1cIm1hcmtcIiAvPlxuICAgIDwvZz5cbiAgKTtcbiAgcmV0dXJuIChcbiAgICA8c3ZnIGNsYXNzTmFtZT1cImNvcm5lci1tYXJrc1wiIHZpZXdCb3g9e2AwIDAgJHtDQU5WQVMud30gJHtofWB9PlxuICAgICAge21hcmsoOCwgOCwgIDI0LCAgMjQpfVxuICAgICAge21hcmsoQ0FOVkFTLncgLSA4LCA4LCAgLTI0LCAgMjQpfVxuICAgICAge21hcmsoOCwgaCAtIDgsICAyNCwgLTI0KX1cbiAgICAgIHttYXJrKENBTlZBUy53IC0gOCwgaCAtIDgsIC0yNCwgLTI0KX1cbiAgICA8L3N2Zz5cbiAgKTtcbn1cblxuZnVuY3Rpb24gUGxhdGVBbm5vdGF0aW9ucyh7IGFjdGl2ZVRyYWNlLCBjYW52YXNIIH0pIHtcbiAgY29uc3QgeyBDQU5WQVMsIE1FVEEgfSA9IHdpbmRvdy5RT0xfRElBR1JBTTtcbiAgY29uc3QgaCA9IGNhbnZhc0ggIT0gbnVsbCA/IGNhbnZhc0ggOiBDQU5WQVMuaDtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRlLWFubm90YXRpb25zXCIgc3R5bGU9e3sgd2lkdGg6IENBTlZBUy53LCBoZWlnaHQ6IGggfX0+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRlIHBsYXRlLXRsXCI+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwicGxhdGUtbGluZVwiPmZpZyDCtyAwMTwvZGl2PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRlLWxpbmUgcGxhdGUtbW9ub1wiPnJ1bnRpbWUgYXJjaGl0ZWN0dXJlIG1hcDwvZGl2PlxuICAgICAgPC9kaXY+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRlIHBsYXRlLXRyXCI+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwicGxhdGUtbGluZVwiPnFvbC10cmF5PC9kaXY+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwicGxhdGUtbGluZSBwbGF0ZS1tb25vXCI+djMuMTUuMSDCtyBtYWluPC9kaXY+XG4gICAgICA8L2Rpdj5cbiAgICAgIDxkaXYgY2xhc3NOYW1lPVwicGxhdGUgcGxhdGUtYmwgcGxhdGUtbW9ub1wiPlxuICAgICAgICB7TUVUQSAmJiBNRVRBLmJpbmFyaWVzfVxuICAgICAgPC9kaXY+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInBsYXRlIHBsYXRlLWJyIHBsYXRlLW1vbm9cIj5cbiAgICAgICAge2FjdGl2ZVRyYWNlID8gYHRyYWNlIMK3ICR7YWN0aXZlVHJhY2Uub3JkfSDCtyAke2FjdGl2ZVRyYWNlLmxhYmVsfWAgOiBcInRyYWNlIMK3IGlkbGVcIn1cbiAgICAgIDwvZGl2PlxuICAgIDwvZGl2PlxuICApO1xufVxuXG4vLyDilIDilIDilIAgdG9wYmFyIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgFxuXG5mdW5jdGlvbiBUb3BiYXIoeyBhY3RpdmVUcmFjZSwgdHJhY2VzLCBvblNlbGVjdFRyYWNlLCBsZXZlbCwgc2V0TGV2ZWwgfSkge1xuICBjb25zdCBbdHJhY2VPcGVuLCBzZXRUcmFjZU9wZW5dID0gdXNlU3RhdGUoZmFsc2UpO1xuICBjb25zdCBtZW51UmVmID0gdXNlUmVmKG51bGwpO1xuXG4gIHVzZUVmZmVjdCgoKSA9PiB7XG4gICAgaWYgKCF0cmFjZU9wZW4pIHJldHVybjtcbiAgICBmdW5jdGlvbiBvbkRvY0NsaWNrKGUpIHtcbiAgICAgIGlmIChtZW51UmVmLmN1cnJlbnQgJiYgIW1lbnVSZWYuY3VycmVudC5jb250YWlucyhlLnRhcmdldCkpIHNldFRyYWNlT3BlbihmYWxzZSk7XG4gICAgfVxuICAgIGRvY3VtZW50LmFkZEV2ZW50TGlzdGVuZXIoXCJtb3VzZWRvd25cIiwgb25Eb2NDbGljayk7XG4gICAgcmV0dXJuICgpID0+IGRvY3VtZW50LnJlbW92ZUV2ZW50TGlzdGVuZXIoXCJtb3VzZWRvd25cIiwgb25Eb2NDbGljayk7XG4gIH0sIFt0cmFjZU9wZW5dKTtcblxuICByZXR1cm4gKFxuICAgIDxkaXYgY2xhc3NOYW1lPVwidG9wYmFyXCI+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInRvcGJhci1sZWZ0XCI+XG4gICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInRvcGJhci10aXRsZVwiPlFvTCBUcmF5PC9zcGFuPlxuICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJ0b3BiYXItc2VwXCI+wrc8L3NwYW4+XG4gICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInRvcGJhci1zdWJcIj5SdW50aW1lIEFyY2hpdGVjdHVyZSBNYXA8L3NwYW4+XG4gICAgICA8L2Rpdj5cblxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJ0b3BiYXItY2VudGVyXCIgcmVmPXttZW51UmVmfT5cbiAgICAgICAgPGJ1dHRvbiBjbGFzc05hbWU9e2B0cmFjZS10b2dnbGUgJHthY3RpdmVUcmFjZSA/IFwiaXMtYWN0aXZlXCIgOiBcIlwifWB9XG4gICAgICAgICAgICAgICAgb25DbGljaz17KCkgPT4gc2V0VHJhY2VPcGVuKChvKSA9PiAhbyl9PlxuICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInRyYWNlLXRvZ2dsZS1pY29cIj7ilrc8L3NwYW4+XG4gICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwidHJhY2UtdG9nZ2xlLWxhYmVsXCI+XG4gICAgICAgICAgICB7YWN0aXZlVHJhY2UgPyA8PnRyYWNlIMK3IDxiPnthY3RpdmVUcmFjZS5vcmR9PC9iPiB7YWN0aXZlVHJhY2UubGFiZWx9PC8+IDogXCJ0cmFjZXNcIn1cbiAgICAgICAgICA8L3NwYW4+XG4gICAgICAgICAge2FjdGl2ZVRyYWNlID8gKFxuICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwidHJhY2UtdG9nZ2xlLXN0b3BcIlxuICAgICAgICAgICAgICAgICAgb25DbGljaz17KGUpID0+IHsgZS5zdG9wUHJvcGFnYXRpb24oKTsgb25TZWxlY3RUcmFjZShhY3RpdmVUcmFjZSk7IH19PsOXPC9zcGFuPlxuICAgICAgICAgICkgOiAoXG4gICAgICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJ0cmFjZS10b2dnbGUtY2FyZXRcIj57dHJhY2VPcGVuID8gXCLilrRcIiA6IFwi4pa+XCJ9PC9zcGFuPlxuICAgICAgICAgICl9XG4gICAgICAgIDwvYnV0dG9uPlxuXG4gICAgICAgIHt0cmFjZU9wZW4gJiYgKFxuICAgICAgICAgIDxkaXYgY2xhc3NOYW1lPVwidHJhY2UtbWVudVwiPlxuICAgICAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJ0cmFjZS1tZW51LWV5ZWJyb3dcIj5ydW50aW1lIHRyYWNlPC9kaXY+XG4gICAgICAgICAgICB7dHJhY2VzLm1hcCgodCkgPT4gKFxuICAgICAgICAgICAgICA8YnV0dG9uIGtleT17dC5pZH1cbiAgICAgICAgICAgICAgICAgICAgICBjbGFzc05hbWU9e2B0cmFjZS1tZW51LWl0ZW0gJHthY3RpdmVUcmFjZSAmJiBhY3RpdmVUcmFjZS5pZCA9PT0gdC5pZCA/IFwiaXMtb25cIiA6IFwiXCJ9ICR7dC50b25lID09PSBcImFtYmVyXCIgPyBcInRvbmUtYW1iZXJcIiA6IFwiXCJ9YH1cbiAgICAgICAgICAgICAgICAgICAgICBvbkNsaWNrPXsoKSA9PiB7IG9uU2VsZWN0VHJhY2UodCk7IHNldFRyYWNlT3BlbihmYWxzZSk7IH19PlxuICAgICAgICAgICAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInRyYWNlLW9yZFwiPnt0Lm9yZH08L3NwYW4+XG4gICAgICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwidHJhY2UtbGFiZWxcIj57dC5sYWJlbH08L3NwYW4+XG4gICAgICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwidHJhY2UtcGxheVwiPnthY3RpdmVUcmFjZSAmJiBhY3RpdmVUcmFjZS5pZCA9PT0gdC5pZCA/IFwi4pagXCIgOiBcIuKWt1wifTwvc3Bhbj5cbiAgICAgICAgICAgICAgPC9idXR0b24+XG4gICAgICAgICAgICApKX1cbiAgICAgICAgICA8L2Rpdj5cbiAgICAgICAgKX1cbiAgICAgIDwvZGl2PlxuXG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInRvcGJhci1yaWdodFwiPlxuICAgICAgICA8c3BhbiBjbGFzc05hbWU9XCJ0b3BiYXItZXllYnJvd1wiPmxldmVsPC9zcGFuPlxuICAgICAgICB7W1wibWluaW1hbFwiLFwiZGVzY3JpcHRpdmVcIixcImRldGFpbGVkXCJdLm1hcCgobCkgPT4gKFxuICAgICAgICAgIDxidXR0b24ga2V5PXtsfSBjbGFzc05hbWU9e2BwaWxsICR7bGV2ZWwgPT09IGwgPyBcImlzLW9uXCIgOiBcIlwifWB9XG4gICAgICAgICAgICAgICAgICBvbkNsaWNrPXsoKSA9PiBzZXRMZXZlbChsKX0+e2x9PC9idXR0b24+XG4gICAgICAgICkpfVxuICAgICAgPC9kaXY+XG4gICAgPC9kaXY+XG4gICk7XG59XG5cbi8vIOKUgOKUgOKUgCBkZXRhaWwgcGFuZWwg4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSAXG5cbi8vIFRyYWNlIG5hcnJhdGl2ZSBzaWRlIHBhbmVsLiBOb2RlIGRldGFpbHMgYXJlIG5vdyBzaG93biBpbmxpbmUgYnkgY2xpY2tpbmdcbi8vIHRoZSBjYXJkIHRvIHRvZ2dsZSBleHBhbnNpb24sIHNvIHRoaXMgcGFuZWwgaXMgdHJhY2Utb25seS5cbmZ1bmN0aW9uIERldGFpbFBhbmVsKHsgYWN0aXZlVHJhY2UsIG9uQ2xvc2UgfSkge1xuICBpZiAoIWFjdGl2ZVRyYWNlKSByZXR1cm4gbnVsbDtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT1cImRldGFpbC1wYW5lbFwiPlxuICAgICAgPGJ1dHRvbiBjbGFzc05hbWU9XCJkZXRhaWwtY2xvc2VcIiBvbkNsaWNrPXtvbkNsb3NlfSBhcmlhLWxhYmVsPVwiY2xvc2VcIj7DlzwvYnV0dG9uPlxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJkZXRhaWwtc2VjdGlvblwiPlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImRldGFpbC1leWVicm93XCI+dHJhY2Ugwrcge2FjdGl2ZVRyYWNlLm9yZH08L2Rpdj5cbiAgICAgICAgPGRpdiBjbGFzc05hbWU9XCJkZXRhaWwtdGl0bGVcIj57YWN0aXZlVHJhY2UubGFiZWx9PC9kaXY+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwiZGV0YWlsLWJvZHlcIj57YWN0aXZlVHJhY2UubmFycmF0aXZlfTwvZGl2PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cImRldGFpbC10cmFjZS1zdGVwc1wiPlxuICAgICAgICAgIHthY3RpdmVUcmFjZS5zdGVwcy5tYXAoKGlkLCBpKSA9PiB7XG4gICAgICAgICAgICBjb25zdCBuID0gbm9kZUJ5SWQoaWQpO1xuICAgICAgICAgICAgcmV0dXJuIChcbiAgICAgICAgICAgICAgPFJlYWN0LkZyYWdtZW50IGtleT17aX0+XG4gICAgICAgICAgICAgICAgPHNwYW4gY2xhc3NOYW1lPVwic3RlcFwiPntuID8gbi5sYWJlbCA6IGlkfTwvc3Bhbj5cbiAgICAgICAgICAgICAgICB7aSA8IGFjdGl2ZVRyYWNlLnN0ZXBzLmxlbmd0aCAtIDEgJiYgPHNwYW4gY2xhc3NOYW1lPVwic3RlcC1zZXBcIj7ihpM8L3NwYW4+fVxuICAgICAgICAgICAgICA8L1JlYWN0LkZyYWdtZW50PlxuICAgICAgICAgICAgKTtcbiAgICAgICAgICB9KX1cbiAgICAgICAgPC9kaXY+XG4gICAgICA8L2Rpdj5cbiAgICA8L2Rpdj5cbiAgKTtcbn1cblxud2luZG93LkRpYWdyYW0gPSBEaWFncmFtO1xuIl0sIm1hcHBpbmdzIjoiQUFBQTtBQUNBO0FBQ0E7O0FBRUEsTUFBTTtFQUFFQSxRQUFRO0VBQUVDLFNBQVM7RUFBRUMsTUFBTTtFQUFFQyxPQUFPO0VBQUVDO0FBQVksQ0FBQyxHQUFHQyxLQUFLOztBQUVuRTs7QUFFQSxTQUFTQyxRQUFRQSxDQUFDQyxFQUFFLEVBQUU7RUFBRSxPQUFPQyxNQUFNLENBQUNDLFdBQVcsQ0FBQ0MsS0FBSyxDQUFDQyxJQUFJLENBQUVDLENBQUMsSUFBS0EsQ0FBQyxDQUFDTCxFQUFFLEtBQUtBLEVBQUUsQ0FBQztBQUFFOztBQUVsRjtBQUNBO0FBQ0E7QUFDQSxTQUFTTSxRQUFRQSxDQUFBLEVBQUc7RUFDbEIsT0FBUUwsTUFBTSxDQUFDQyxXQUFXLElBQUlELE1BQU0sQ0FBQ0MsV0FBVyxDQUFDSyxJQUFJLElBQUssQ0FBQyxDQUFDO0FBQzlEOztBQUVBO0FBQ0E7QUFDQSxTQUFTQyxTQUFTQSxDQUFDQyxDQUFDLEVBQUVDLENBQUMsRUFBRTtFQUN2QixNQUFNQyxH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
})();

// AUTO-GENERATED from the matching .jsx file via build.mjs. Edit the .jsx, then run `npm run build`.
(function () {
"use strict";
// tweaks-panel.jsx
// Reusable Tweaks shell + form-control helpers.
//
// Owns the host protocol (listens for __activate_edit_mode / __deactivate_edit_mode,
// posts __edit_mode_available / __edit_mode_set_keys / __edit_mode_dismissed) so
// individual prototypes don't re-roll it. Ships a consistent set of controls so you
// don't hand-draw <input type="range">, segmented radios, steppers, etc.
//
// Usage (in an HTML file that loads React + Babel):
//
//   const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
//     "primaryColor": "#D97757",
//     "palette": ["#D97757", "#29261b", "#f6f4ef"],
//     "fontSize": 16,
//     "density": "regular",
//     "dark": false
//   }/*EDITMODE-END*/;
//
//   function App() {
//     const [t, setTweak] = useTweaks(TWEAK_DEFAULTS);
//     return (
//       <div style={{ fontSize: t.fontSize, color: t.primaryColor }}>
//         Hello
//         <TweaksPanel>
//           <TweakSection label="Typography" />
//           <TweakSlider label="Font size" value={t.fontSize} min={10} max={32} unit="px"
//                        onChange={(v) => setTweak('fontSize', v)} />
//           <TweakRadio  label="Density" value={t.density}
//                        options={['compact', 'regular', 'comfy']}
//                        onChange={(v) => setTweak('density', v)} />
//           <TweakSection label="Theme" />
//           <TweakColor  label="Primary" value={t.primaryColor}
//                        options={['#D97757', '#2A6FDB', '#1F8A5B', '#7A5AE0']}
//                        onChange={(v) => setTweak('primaryColor', v)} />
//           <TweakColor  label="Palette" value={t.palette}
//                        options={[['#D97757', '#29261b', '#f6f4ef'],
//                                  ['#475569', '#0f172a', '#f1f5f9']]}
//                        onChange={(v) => setTweak('palette', v)} />
//           <TweakToggle label="Dark mode" value={t.dark}
//                        onChange={(v) => setTweak('dark', v)} />
//         </TweaksPanel>
//       </div>
//     );
//   }
//
// ─────────────────────────────────────────────────────────────────────────────

const __TWEAKS_STYLE = `
  .twk-panel{position:fixed;right:16px;bottom:16px;z-index:2147483646;width:280px;
    max-height:calc(100vh - 32px);display:flex;flex-direction:column;
    transform:scale(var(--dc-inv-zoom,1));transform-origin:bottom right;
    background:rgba(250,249,247,.78);color:#29261b;
    -webkit-backdrop-filter:blur(24px) saturate(160%);backdrop-filter:blur(24px) saturate(160%);
    border:.5px solid rgba(255,255,255,.6);border-radius:14px;
    box-shadow:0 1px 0 rgba(255,255,255,.5) inset,0 12px 40px rgba(0,0,0,.18);
    font:11.5px/1.4 ui-sans-serif,system-ui,-apple-system,sans-serif;overflow:hidden}
  .twk-hd{display:flex;align-items:center;justify-content:space-between;
    padding:10px 8px 10px 14px;cursor:move;user-select:none}
  .twk-hd b{font-size:12px;font-weight:600;letter-spacing:.01em}
  .twk-x{appearance:none;border:0;background:transparent;color:rgba(41,38,27,.55);
    width:22px;height:22px;border-radius:6px;cursor:default;font-size:13px;line-height:1}
  .twk-x:hover{background:rgba(0,0,0,.06);color:#29261b}
  .twk-body{padding:2px 14px 14px;display:flex;flex-direction:column;gap:10px;
    overflow-y:auto;overflow-x:hidden;min-height:0;
    scrollbar-width:thin;scrollbar-color:rgba(0,0,0,.15) transparent}
  .twk-body::-webkit-scrollbar{width:8px}
  .twk-body::-webkit-scrollbar-track{background:transparent;margin:2px}
  .twk-body::-webkit-scrollbar-thumb{background:rgba(0,0,0,.15);border-radius:4px;
    border:2px solid transparent;background-clip:content-box}
  .twk-body::-webkit-scrollbar-thumb:hover{background:rgba(0,0,0,.25);
    border:2px solid transparent;background-clip:content-box}
  .twk-row{display:flex;flex-direction:column;gap:5px}
  .twk-row-h{flex-direction:row;align-items:center;justify-content:space-between;gap:10px}
  .twk-lbl{display:flex;justify-content:space-between;align-items:baseline;
    color:rgba(41,38,27,.72)}
  .twk-lbl>span:first-child{font-weight:500}
  .twk-val{color:rgba(41,38,27,.5);font-variant-numeric:tabular-nums}

  .twk-sect{font-size:10px;font-weight:600;letter-spacing:.06em;text-transform:uppercase;
    color:rgba(41,38,27,.45);padding:10px 0 0}
  .twk-sect:first-child{padding-top:0}

  .twk-field{appearance:none;box-sizing:border-box;width:100%;min-width:0;height:26px;padding:0 8px;
    border:.5px solid rgba(0,0,0,.1);border-radius:7px;
    background:rgba(255,255,255,.6);color:inherit;font:inherit;outline:none}
  .twk-field:focus{border-color:rgba(0,0,0,.25);background:rgba(255,255,255,.85)}
  select.twk-field{padding-right:22px;
    background-image:url("data:image/svg+xml;utf8,<svg xmlns='http://www.w3.org/2000/svg' width='10' height='6' viewBox='0 0 10 6'><path fill='rgba(0,0,0,.5)' d='M0 0h10L5 6z'/></svg>");
    background-repeat:no-repeat;background-position:right 8px center}

  .twk-slider{appearance:none;-webkit-appearance:none;width:100%;height:4px;margin:6px 0;
    border-radius:999px;background:rgba(0,0,0,.12);outline:none}
  .twk-slider::-webkit-slider-thumb{-webkit-appearance:none;appearance:none;
    width:14px;height:14px;border-radius:50%;background:#fff;
    border:.5px solid rgba(0,0,0,.12);box-shadow:0 1px 3px rgba(0,0,0,.2);cursor:default}
  .twk-slider::-moz-range-thumb{width:14px;height:14px;border-radius:50%;
    background:#fff;border:.5px solid rgba(0,0,0,.12);box-shadow:0 1px 3px rgba(0,0,0,.2);cursor:default}

  .twk-seg{position:relative;display:flex;padding:2px;border-radius:8px;
    background:rgba(0,0,0,.06);user-select:none}
  .twk-seg-thumb{position:absolute;top:2px;bottom:2px;border-radius:6px;
    background:rgba(255,255,255,.9);box-shadow:0 1px 2px rgba(0,0,0,.12);
    transition:left .15s cubic-bezier(.3,.7,.4,1),width .15s}
  .twk-seg.dragging .twk-seg-thumb{transition:none}
  .twk-seg button{appearance:none;position:relative;z-index:1;flex:1;border:0;
    background:transparent;color:inherit;font:inherit;font-weight:500;min-height:22px;
    border-radius:6px;cursor:default;padding:4px 6px;line-height:1.2;
    overflow-wrap:anywhere}

  .twk-toggle{position:relative;width:32px;height:18px;border:0;border-radius:999px;
    background:rgba(0,0,0,.15);transition:background .15s;cursor:default;padding:0}
  .twk-toggle[data-on="1"]{background:#34c759}
  .twk-toggle i{position:absolute;top:2px;left:2px;width:14px;height:14px;border-radius:50%;
    background:#fff;box-shadow:0 1px 2px rgba(0,0,0,.25);transition:transform .15s}
  .twk-toggle[data-on="1"] i{transform:translateX(14px)}

  .twk-num{display:flex;align-items:center;box-sizing:border-box;min-width:0;height:26px;padding:0 0 0 8px;
    border:.5px solid rgba(0,0,0,.1);border-radius:7px;background:rgba(255,255,255,.6)}
  .twk-num-lbl{font-weight:500;color:rgba(41,38,27,.6);cursor:ew-resize;
    user-select:none;padding-right:8px}
  .twk-num input{flex:1;min-width:0;height:100%;border:0;background:transparent;
    font:inherit;font-variant-numeric:tabular-nums;text-align:right;padding:0 8px 0 0;
    outline:none;color:inherit;-moz-appearance:textfield}
  .twk-num input::-webkit-inner-spin-button,.twk-num input::-webkit-outer-spin-button{
    -webkit-appearance:none;margin:0}
  .twk-num-unit{padding-right:8px;color:rgba(41,38,27,.45)}

  .twk-btn{appearance:none;height:26px;padding:0 12px;border:0;border-radius:7px;
    background:rgba(0,0,0,.78);color:#fff;font:inherit;font-weight:500;cursor:default}
  .twk-btn:hover{background:rgba(0,0,0,.88)}
  .twk-btn.secondary{background:rgba(0,0,0,.06);color:inherit}
  .twk-btn.secondary:hover{background:rgba(0,0,0,.1)}

  .twk-swatch{appearance:none;-webkit-appearance:none;width:56px;height:22px;
    border:.5px solid rgba(0,0,0,.1);border-radius:6px;padding:0;cursor:default;
    background:transparent;flex-shrink:0}
  .twk-swatch::-webkit-color-swatch-wrapper{padding:0}
  .twk-swatch::-webkit-color-swatch{border:0;border-radius:5.5px}
  .twk-swatch::-moz-color-swatch{border:0;border-radius:5.5px}

  .twk-chips{display:flex;gap:6px}
  .twk-chip{position:relative;appearance:none;flex:1;min-width:0;height:46px;
    padding:0;border:0;border-radius:6px;overflow:hidden;cursor:default;
    box-shadow:0 0 0 .5px rgba(0,0,0,.12),0 1px 2px rgba(0,0,0,.06);
    transition:transform .12s cubic-bezier(.3,.7,.4,1),box-shadow .12s}
  .twk-chip:hover{transform:translateY(-1px);
    box-shadow:0 0 0 .5px rgba(0,0,0,.18),0 4px 10px rgba(0,0,0,.12)}
  .twk-chip[data-on="1"]{box-shadow:0 0 0 1.5px rgba(0,0,0,.85),
    0 2px 6px rgba(0,0,0,.15)}
  .twk-chip>span{position:absolute;top:0;bottom:0;right:0;width:34%;
    display:flex;flex-direction:column;box-shadow:-1px 0 0 rgba(0,0,0,.1)}
  .twk-chip>span>i{flex:1;box-shadow:0 -1px 0 rgba(0,0,0,.1)}
  .twk-chip>span>i:first-child{box-shadow:none}
  .twk-chip svg{position:absolute;top:6px;left:6px;width:13px;height:13px;
    filter:drop-shadow(0 1px 1px rgba(0,0,0,.3))}
`;

// ── useTweaks ───────────────────────────────────────────────────────────────
// Single source of truth for tweak values. setTweak persists via the host
// (__edit_mode_set_keys → host rewrites the EDITMODE block on disk).
function useTweaks(defaults) {
  const [values, setValues] = React.useState(defaults);
  // Accepts either setTweak('key', value) or setTweak({ key: value, ... }) so a
  // useState-style call doesn't write a "[object Object]" key into the persisted
  // JSON block.
  const setTweak = React.useCallback((keyOrEdits, val) => {
    const edits = typeof keyOrEdits === 'object' && keyOrEdits !== null ? keyOrEdits : {
      [keyOrEdits]: val
    };
    setValues(prev => ({
      ...prev,
      ...edits
    }));
    window.parent.postMessage({
      type: '__edit_mode_set_keys',
      edits
    }, '*');
    // Same-window signal so in-page listeners (deck-stage rail thumbnails)
    // can react — the parent message only reaches the host, not peers.
    window.dispatchEvent(new CustomEvent('tweakchange', {
      detail: edits
    }));
  }, []);
  return [values, setTweak];
}

// ── TweaksPanel ─────────────────────────────────────────────────────────────
// Floating shell. Registers the protocol listener BEFORE announcing
// availability — if the announce ran first, the host's activate could land
// before our handler exists and the toolbar toggle would silently no-op.
// The close button posts __edit_mode_dismissed so the host's toolbar toggle
// flips off in lockstep; the host echoes __deactivate_edit_mode back which
// is what actually hides the panel.
function TweaksPanel({
  title = 'Tweaks',
  noDeckControls = false,
  children
}) {
  const [open, setOpen] = React.useState(false);
  const dragRef = React.useRef(null);
  // Auto-inject a rail toggle when a <deck-stage> is on the page. The
  // toggle drives the deck's per-viewer _railVisible via window message;
  // state is mirrored from the same localStorage key the deck reads so
  // the control reflects reality across reloads. The mechanism is the
  // message — authors who want custom placement can post it directly
  // and pass noDeckControls to suppress this one.
  const hasDeckStage = React.useMemo(() => typeof document !== 'undefined' && !!document.querySelector('deck-stage'), []);
  // deck-stage enables its rail in connectedCallback, but this panel can
  // mount before that element has upgraded. The initial read catches the
  // common case; the listener covers mounting first. (Older deck-stage.js
  // copies still wait for the host's __omelette_rail_enabled postMessage —
  // same listener handles those.)
  const [railEnabled, setRailEnabled] = React.useState(() => hasDeckStage && !!document.querySelector('deck-stage')?._railEnabled);
  React.useEffect(() => {
    if (!hasDeckStage || railEnabled) return undefined;
    const onMsg = e => {
      if (e.data && e.data.type === '__omelette_rail_enabled') setRailEnabled(true);
    };
    window.addEventListener('message', onMsg);
    return () => window.removeEventListener('message', onMsg);
  }, [hasDeckStage, railEnabled]);
  const [railVisible, setRailVisible] = React.useState(() => {
    try {
      return localStorage.getItem('deck-stage.railVisible') !== '0';
    } catch (e) {
      return true;
    }
  });
  const toggleRail = on => {
    setRailVisible(on);
    window.postMessage({
      type: '__deck_rail_visible',
      on
    }, '*');
  };
  const offsetRef = React.useRef({
    x: 16,
    y: 16
  });
  const PAD = 16;
  const clampToViewport = React.useCallback(() => {
    const panel = dragRef.current;
    if (!panel) return;
    const w = panel.offsetWidth,
      h = panel.offsetHeight;
    const maxRight = Math.max(PAD, window.innerWidth - w - PAD);
    const maxBottom = Math.max(PAD, window.innerHeight - h - PAD);
    offsetRef.current = {
      x: Math.min(maxRight, Math.max(PAD, offsetRef.current.x)),
      y: Math.min(maxBottom, Math.max(PAD, offsetRef.current.y))
    };
    panel.style.right = offsetRef.current.x + 'px';
    panel.style.bottom = offsetRef.current.y + 'px';
  }, []);
  React.useEffect(() => {
    if (!open) return;
    clampToViewport();
    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', clampToViewport);
      return () => window.removeEventListener('resize', clampToViewport);
    }
    const ro = new ResizeObserver(clampToViewport);
    ro.observe(document.documentElement);
    return () => ro.disconnect();
  }, [open, clampToViewport]);
  React.useEffect(() => {
    const onMsg = e => {
      const t = e?.data?.type;
      if (t === '__activate_edit_mode') setOpen(true);else if (t === '__deactivate_edit_mode') setOpen(false);
    };
    window.addEventListener('message', onMsg);
    window.parent.postMessage({
      type: '__edit_mode_available'
    }, '*');
    return () => window.removeEventListener('message', onMsg);
  }, []);
  const dismiss = () => {
    setOpen(false);
    window.parent.postMessage({
      type: '__edit_mode_dismissed'
    }, '*');
  };
  const onDragStart = e => {
    const panel = dragRef.current;
    if (!panel) return;
    const r = panel.getBoundingClientRect();
    const sx = e.clientX,
      sy = e.clientY;
    const startRight = window.innerWidth - r.right;
    const startBottom = window.innerHeight - r.bottom;
    const move = ev => {
      offsetRef.current = {
        x: startRight - (ev.clientX - sx),
        y: startBottom - (ev.clientY - sy)
      };
      clampToViewport();
    };
    const up = () => {
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  };
  if (!open) return null;
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("style", null, __TWEAKS_STYLE), /*#__PURE__*/React.createElement("div", {
    ref: dragRef,
    className: "twk-panel",
    "data-noncommentable": "",
    style: {
      right: offsetRef.current.x,
      bottom: offsetRef.current.y
    }
  }, /*#__PURE__*/React.createElement("div", {
    className: "twk-hd",
    onMouseDown: onDragStart
  }, /*#__PURE__*/React.createElement("b", null, title), /*#__PURE__*/React.createElement("button", {
    className: "twk-x",
    "aria-label": "Close tweaks",
    onMouseDown: e => e.stopPropagation(),
    onClick: dismiss
  }, "\u2715")), /*#__PURE__*/React.createElement("div", {
    className: "twk-body"
  }, children, hasDeckStage && railEnabled && !noDeckControls && /*#__PURE__*/React.createElement(TweakSection, {
    label: "Deck"
  }, /*#__PURE__*/React.createElement(TweakToggle, {
    label: "Thumbnail rail",
    value: railVisible,
    onChange: toggleRail
  })))));
}

// ── Layout helpers ──────────────────────────────────────────────────────────

function TweakSection({
  label,
  children
}) {
  return /*#__PURE__*/React.createElement(React.Fragment, null, /*#__PURE__*/React.createElement("div", {
    className: "twk-sect"
  }, label), children);
}
function TweakRow({
  label,
  value,
  children,
  inline = false
}) {
  return /*#__PURE__*/React.createElement("div", {
    className: inline ? 'twk-row twk-row-h' : 'twk-row'
  }, /*#__PURE__*/React.createElement("div", {
    className: "twk-lbl"
  }, /*#__PURE__*/React.createElement("span", null, label), value != null && /*#__PURE__*/React.createElement("span", {
    className: "twk-val"
  }, value)), children);
}

// ── Controls ────────────────────────────────────────────────────────────────

function TweakSlider({
  label,
  value,
  min = 0,
  max = 100,
  step = 1,
  unit = '',
  onChange
}) {
  return /*#__PURE__*/React.createElement(TweakRow, {
    label: label,
    value: `${value}${unit}`
  }, /*#__PURE__*/React.createElement("input", {
    type: "range",
    className: "twk-slider",
    min: min,
    max: max,
    step: step,
    value: value,
    onChange: e => onChange(Number(e.target.value))
  }));
}
function TweakToggle({
  label,
  value,
  onChange
}) {
  return /*#__PURE__*/React.createElement("div", {
    className: "twk-row twk-row-h"
  }, /*#__PURE__*/React.createElement("div", {
    className: "twk-lbl"
  }, /*#__PURE__*/React.createElement("span", null, label)), /*#__PURE__*/React.createElement("button", {
    type: "button",
    className: "twk-toggle",
    "data-on": value ? '1' : '0',
    role: "switch",
    "aria-checked": !!value,
    onClick: () => onChange(!value)
  }, /*#__PURE__*/React.createElement("i", null)));
}
function TweakRadio({
  label,
  value,
  options,
  onChange
}) {
  const trackRef = React.useRef(null);
  const [dragging, setDragging] = React.useState(false);
  // The active value is read by pointer-move handlers attached for the lifetime
  // of a drag — ref it so a stale closure doesn't fire onChange for every move.
  const valueRef = React.useRef(value);
  valueRef.current = value;

  // Segments wrap mid-word once per-segment width runs out. The track is
  // ~248px (280 panel − 28 body pad − 4 seg pad), each button loses 12px
  // to its own padding, and 11.5px system-ui averages ~6.3px/char — so 2
  // options fit ~16 chars each, 3 fit ~10. Past that (or >3 options), fall
  // back to a dropdown rather than wrap.
  const labelLen = o => String(typeof o === 'object' ? o.label : o).length;
  const maxLen = options.reduce((m, o) => Math.max(m, labelLen(o)), 0);
  const fitsAsSegments = maxLen <= ({
    2: 16,
    3: 10
  }[options.length] ?? 0);
  if (!fitsAsSegments) {
    // <select> emits strings — map back to the original option value so the
    // fallback stays type-preserving (numbers, booleans) like the segment path.
    const resolve = s => {
      const m = options.find(o => String(typeof o === 'object' ? o.value : o) === s);
      return m === undefined ? s : typeof m === 'object' ? m.value : m;
    };
    return /*#__PURE__*/React.createElement(TweakSelect, {
      label: label,
      value: value,
      options: options,
      onChange: s => onChange(resolve(s))
    });
  }
  const opts = options.map(o => typeof o === 'object' ? o : {
    value: o,
    label: o
  });
  const idx = Math.max(0, opts.findIndex(o => o.value === value));
  const n = opts.length;
  const segAt = clientX => {
    const r = trackRef.current.getBoundingClientRect();
    const inner = r.width - 4;
    const i = Math.floor((clientX - r.left - 2) / inner * n);
    return opts[Math.max(0, Math.min(n - 1, i))].value;
  };
  const onPointerDown = e => {
    setDragging(true);
    const v0 = segAt(e.clientX);
    if (v0 !== valueRef.current) onChange(v0);
    const move = ev => {
      if (!trackRef.current) return;
      const v = segAt(ev.clientX);
      if (v !== valueRef.current) onChange(v);
    };
    const up = () => {
      setDragging(false);
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return /*#__PURE__*/React.createElement(TweakRow, {
    label: label
  }, /*#__PURE__*/React.createElement("div", {
    ref: trackRef,
    role: "radiogroup",
    onPointerDown: onPointerDown,
    className: dragging ? 'twk-seg dragging' : 'twk-seg'
  }, /*#__PURE__*/React.createElement("div", {
    className: "twk-seg-thumb",
    style: {
      left: `calc(2px + ${idx} * (100% - 4px) / ${n})`,
      width: `calc((100% - 4px) / ${n})`
    }
  }), opts.map(o => /*#__PURE__*/React.createElement("button", {
    key: o.value,
    type: "button",
    role: "radio",
    "aria-checked": o.value === value
  }, o.label))));
}
function TweakSelect({
  label,
  value,
  options,
  onChange
}) {
  return /*#__PURE__*/React.createElement(TweakRow, {
    label: label
  }, /*#__PURE__*/React.createElement("select", {
    className: "twk-field",
    value: value,
    onChange: e => onChange(e.target.value)
  }, options.map(o => {
    const v = typeof o === 'object' ? o.value : o;
    const l = typeof o === 'object' ? o.label : o;
    return /*#__PURE__*/React.createElement("option", {
      key: v,
      value: v
    }, l);
  })));
}
function TweakText({
  label,
  value,
  placeholder,
  onChange
}) {
  return /*#__PURE__*/React.createElement(TweakRow, {
    label: label
  }, /*#__PURE__*/React.createElement("input", {
    className: "twk-field",
    type: "text",
    value: value,
    placeholder: placeholder,
    onChange: e => onChange(e.target.value)
  }));
}
function TweakNumber({
  label,
  value,
  min,
  max,
  step = 1,
  unit = '',
  onChange
}) {
  const clamp = n => {
    if (min != null && n < min) return min;
    if (max != null && n > max) return max;
    return n;
  };
  const startRef = React.useRef({
    x: 0,
    val: 0
  });
  const onScrubStart = e => {
    e.preventDefault();
    startRef.current = {
      x: e.clientX,
      val: value
    };
    const decimals = (String(step).split('.')[1] || '').length;
    const move = ev => {
      const dx = ev.clientX - startRef.current.x;
      const raw = startRef.current.val + dx * step;
      const snapped = Math.round(raw / step) * step;
      onChange(clamp(Number(snapped.toFixed(decimals))));
    };
    const up = () => {
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  };
  return /*#__PURE__*/React.createElement("div", {
    className: "twk-num"
  }, /*#__PURE__*/React.createElement("span", {
    className: "twk-num-lbl",
    onPointerDown: onScrubStart
  }, label), /*#__PURE__*/React.createElement("input", {
    type: "number",
    value: value,
    min: min,
    max: max,
    step: step,
    onChange: e => onChange(clamp(Number(e.target.value)))
  }), unit && /*#__PURE__*/React.createElement("span", {
    className: "twk-num-unit"
  }, unit));
}

// Relative-luminance contrast pick — checkmarks drawn over a swatch need to
// read on both #111 and #fafafa without per-option configuration. Hex input
// only (#rgb / #rrggbb); named or rgb()/hsl() colors fall through to "light".
function __twkIsLight(hex) {
  const h = String(hex).replace('#', '');
  const x = h.length === 3 ? h.replace(/./g, c => c + c) : h.padEnd(6, '0');
  const n = parseInt(x.slice(0, 6), 16);
  if (Number.isNaN(n)) return true;
  const r = n >> 16 & 255,
    g = n >> 8 & 255,
    b = n & 255;
  return r * 299 + g * 587 + b * 114 > 148000;
}
const __TwkCheck = ({
  light
}) => /*#__PURE__*/React.createElement("svg", {
  viewBox: "0 0 14 14",
  "aria-hidden": "true"
}, /*#__PURE__*/React.createElement("path", {
  d: "M3 7.2 5.8 10 11 4.2",
  fill: "none",
  strokeWidth: "2.2",
  strokeLinecap: "round",
  strokeLinejoin: "round",
  stroke: light ? 'rgba(0,0,0,.78)' : '#fff'
}));

// TweakColor — curated color/palette picker. Each option is either a single
// hex string or an array of 1-5 hex strings; the card adapts — a lone color
// renders solid, a palette renders colors[0] as the hero (left ~2/3) with the
// rest stacked in a sharp column on the right. onChange emits the
// option in the shape it was passed (string stays string, array stays array).
// Without options it falls back to the native color input for back-compat.
function TweakColor({
  label,
  value,
  options,
  onChange
}) {
  if (!options || !options.length) {
    return /*#__PURE__*/React.createElement("div", {
      className: "twk-row twk-row-h"
    }, /*#__PURE__*/React.createElement("div", {
      className: "twk-lbl"
    }, /*#__PURE__*/React.createElement("span", null, label)), /*#__PURE__*/React.createElement("input", {
      type: "color",
      className: "twk-swatch",
      value: value,
      onChange: e => onChange(e.target.value)
    }));
  }
  // Native <input type=color> emits lowercase hex per the HTML spec, so
  // compare case-insensitively. String() guards JSON.stringify(undefined),
  // which returns the primitive undefined (no .toLowerCase).
  const key = o => String(JSON.stringify(o)).toLowerCase();
  const cur = key(value);
  return /*#__PURE__*/React.createElement(TweakRow, {
    label: label
  }, /*#__PURE__*/React.createElement("div", {
    className: "twk-chips",
    role: "radiogroup"
  }, options.map((o, i) => {
    const colors = Array.isArray(o) ? o : [o];
    const [hero, ...rest] = colors;
    const sup = rest.slice(0, 4);
    const on = key(o) === cur;
    return /*#__PURE__*/React.createElement("button", {
      key: i,
      type: "button",
      className: "twk-chip",
      role: "radio",
      "aria-checked": on,
      "data-on": on ? '1' : '0',
      "aria-label": colors.join(', '),
      title: colors.join(' · '),
      style: {
        background: hero
      },
      onClick: () => onChange(o)
    }, sup.length > 0 && /*#__PURE__*/React.createElement("span", null, sup.map((c, j) => /*#__PURE__*/React.createElement("i", {
      key: j,
      style: {
        background: c
      }
    }))), on && /*#__PURE__*/React.createElement(__TwkCheck, {
      light: __twkIsLight(hero)
    }));
  })));
}
function TweakButton({
  label,
  onClick,
  secondary = false
}) {
  return /*#__PURE__*/React.createElement("button", {
    type: "button",
    className: secondary ? 'twk-btn secondary' : 'twk-btn',
    onClick: onClick
  }, label);
}
Object.assign(window, {
  useTweaks,
  TweaksPanel,
  TweakSection,
  TweakRow,
  TweakSlider,
  TweakToggle,
  TweakRadio,
  TweakSelect,
  TweakText,
  TweakNumber,
  TweakColor,
  TweakButton
});
//# sourceMappingURL=data:application/json;charset=utf-8;base64,eyJ2ZXJzaW9uIjozLCJuYW1lcyI6WyJfX1RXRUFLU19TVFlMRSIsInVzZVR3ZWFrcyIsImRlZmF1bHRzIiwidmFsdWVzIiwic2V0VmFsdWVzIiwiUmVhY3QiLCJ1c2VTdGF0ZSIsInNldFR3ZWFrIiwidXNlQ2FsbGJhY2siLCJrZXlPckVkaXRzIiwidmFsIiwiZWRpdHMiLCJwcmV2Iiwid2luZG93IiwicGFyZW50IiwicG9zdE1lc3NhZ2UiLCJ0eXBlIiwiZGlzcGF0Y2hFdmVudCIsIkN1c3RvbUV2ZW50IiwiZGV0YWlsIiwiVHdlYWtzUGFuZWwiLCJ0aXRsZSIsIm5vRGVja0NvbnRyb2xzIiwiY2hpbGRyZW4iLCJvcGVuIiwic2V0T3BlbiIsImRyYWdSZWYiLCJ1c2VSZWYiLCJoYXNEZWNrU3RhZ2UiLCJ1c2VNZW1vIiwiZG9jdW1lbnQiLCJxdWVyeVNlbGVjdG9yIiwicmFpbEVuYWJsZWQiLCJzZXRSYWlsRW5hYmxlZCIsIl9yYWlsRW5hYmxlZCIsInVzZUVmZmVjdCIsInVuZGVmaW5lZCIsIm9uTXNnIiwiZSIsImRhdGEiLCJhZGRFdmVudExpc3RlbmVyIiwicmVtb3ZlRXZlbnRMaXN0ZW5lciIsInJhaWxWaXNpYmxlIiwic2V0UmFpbFZpc2libGUiLCJsb2NhbFN0b3JhZ2UiLCJnZXRJdGVtIiwidG9nZ2xlUmFpbCIsIm9uIiwib2Zmc2V0UmVmIiwieCIsInkiLCJQQUQiLCJjbGFtcFRvVmlld3BvcnQiLCJwYW5lbCIsImN1cnJlbnQiLCJ3Iiwib2Zmc2V0V2lkdGgiLCJoIiwib2Zmc2V0SGVpZ2h0IiwibWF4UmlnaHQiLCJNYXRoIiwibWF4IiwiaW5uZXJXaWR0aCIsIm1heEJvdHRvbSIsImlubmVySGVpZ2h0IiwibWluIiwic3R5bGUiLCJyaWdodCIsImJvdHRvbSIsIlJlc2l6ZU9ic2VydmVyIiwicm8iLCJvYnNlcnZlIiwiZG9jdW1lbnRFbGVtZW50IiwiZGlzY29ubmVjdCIsInQiLCJkaXNtaXNzIiwib25EcmFnU3RhcnQiLCJyIiwiZ2V0Qm91bmRpbmdDbGllbnRSZWN0Iiwic3giLCJjbGllbnRYIiwic3kiLCJjbGllbnRZIiwic3RhcnRSaWdodCIsInN0YXJ0Qm90dG9tIiwibW92ZSIsImV2IiwidXAiLCJjcmVhdGVFbGVtZW50IiwiRnJhZ21lbnQiLCJyZWYiLCJjbGFzc05hbWUiLCJvbk1vdXNlRG93biIsInN0b3BQcm9wYWdhdGlvbiIsIm9uQ2xpY2siLCJUd2Vha1NlY3Rpb24iLCJsYWJlbCIsIlR3ZWFrVG9nZ2xlIiwidmFsdWUiLCJvbkNoYW5nZSIsIlR3ZWFrUm93IiwiaW5saW5lIiwiVHdlYWtTbGlkZXIiLCJzdGVwIiwidW5pdCIsIk51bWJlciIsInRhcmdldCIsInJvbGUiLCJUd2Vha1JhZGlvIiwib3B0aW9ucyIsInRyYWNrUmVmIiwiZHJhZ2dpbmciLCJzZXREcmFnZ2luZyIsInZhbHVlUmVmIiwibGFiZWxMZW4iLCJvIiwiU3RyaW5nIiwibGVuZ3RoIiwibWF4TGVuIiwicmVkdWNlIiwibSIsImZpdHNBc1NlZ21lbnRzIiwicmVzb2x2ZSIsInMiLCJmaW5kIiwiVHdlYWtTZWxlY3QiLCJvcHRzIiwibWFwIiwiaWR4IiwiZmluZEluZGV4IiwibiIsInNlZ0F0IiwiaW5uZXIiLCJ3aWR0aCIsImkiLCJmbG9vciIsImxlZnQiLCJvblBvaW50ZXJEb3duIiwidjAiLCJ2Iiwia2V5IiwibCIsIlR3ZWFrVGV4dCIsInBsYWNlaG9sZGVyIiwiVHdlYWtOdW1iZXIiLCJjbGFtcCIsInN0YXJ0UmVmIiwib25TY3J1YlN0YXJ0IiwicHJldmVudERlZmF1bHQiLCJkZWNpbWFscyIsInNwbGl0IiwiZHgiLCJyYXciLCJzbmFwcGVkIiwicm91bmQiLCJ0b0ZpeGVkIiwiX190d2tJc0xpZ2h0IiwiaGV4IiwicmVwbGFjZSIsImMiLCJwYWRFbmQiLCJwYXJzZUludCIsInNsaWNlIiwiaXNOYU4iLCJnIiwiYiIsIl9fVHdrQ2hlY2siLCJsaWdodCIsInZpZXdCb3giLCJkIiwiZmlsbCIsInN0cm9rZVdpZHRoIiwic3Ryb2tlTGluZWNhcCIsInN0cm9rZUxpbmVqb2luIiwic3Ryb2tlIiwiVHdlYWtDb2xvciIsIkpTT04iLCJzdHJpbmdpZnkiLCJ0b0xvd2VyQ2FzZSIsImN1ciIsImNvbG9ycyIsIkFycmF5IiwiaXNBcnJheSIsImhlcm8iLCJyZXN0Iiwic3VwIiwiam9pbiIsImJhY2tncm91bmQiLCJqIiwiVHdlYWtCdXR0b24iLCJzZWNvbmRhcnkiLCJPYmplY3QiLCJhc3NpZ24iXSwic291cmNlcyI6WyJ0d2Vha3MtcGFuZWwuanN4Il0sInNvdXJjZXNDb250ZW50IjpbIlxuLy8gdHdlYWtzLXBhbmVsLmpzeFxuLy8gUmV1c2FibGUgVHdlYWtzIHNoZWxsICsgZm9ybS1jb250cm9sIGhlbHBlcnMuXG4vL1xuLy8gT3ducyB0aGUgaG9zdCBwcm90b2NvbCAobGlzdGVucyBmb3IgX19hY3RpdmF0ZV9lZGl0X21vZGUgLyBfX2RlYWN0aXZhdGVfZWRpdF9tb2RlLFxuLy8gcG9zdHMgX19lZGl0X21vZGVfYXZhaWxhYmxlIC8gX19lZGl0X21vZGVfc2V0X2tleXMgLyBfX2VkaXRfbW9kZV9kaXNtaXNzZWQpIHNvXG4vLyBpbmRpdmlkdWFsIHByb3RvdHlwZXMgZG9uJ3QgcmUtcm9sbCBpdC4gU2hpcHMgYSBjb25zaXN0ZW50IHNldCBvZiBjb250cm9scyBzbyB5b3Vcbi8vIGRvbid0IGhhbmQtZHJhdyA8aW5wdXQgdHlwZT1cInJhbmdlXCI+LCBzZWdtZW50ZWQgcmFkaW9zLCBzdGVwcGVycywgZXRjLlxuLy9cbi8vIFVzYWdlIChpbiBhbiBIVE1MIGZpbGUgdGhhdCBsb2FkcyBSZWFjdCArIEJhYmVsKTpcbi8vXG4vLyAgIGNvbnN0IFRXRUFLX0RFRkFVTFRTID0gLypFRElUTU9ERS1CRUdJTiove1xuLy8gICAgIFwicHJpbWFyeUNvbG9yXCI6IFwiI0Q5Nzc1N1wiLFxuLy8gICAgIFwicGFsZXR0ZVwiOiBbXCIjRDk3NzU3XCIsIFwiIzI5MjYxYlwiLCBcIiNmNmY0ZWZcIl0sXG4vLyAgICAgXCJmb250U2l6ZVwiOiAxNixcbi8vICAgICBcImRlbnNpdHlcIjogXCJyZWd1bGFyXCIsXG4vLyAgICAgXCJkYXJrXCI6IGZhbHNlXG4vLyAgIH0vKkVESVRNT0RFLUVORCovO1xuLy9cbi8vICAgZnVuY3Rpb24gQXBwKCkge1xuLy8gICAgIGNvbnN0IFt0LCBzZXRUd2Vha10gPSB1c2VUd2Vha3MoVFdFQUtfREVGQVVMVFMpO1xuLy8gICAgIHJldHVybiAoXG4vLyAgICAgICA8ZGl2IHN0eWxlPXt7IGZvbnRTaXplOiB0LmZvbnRTaXplLCBjb2xvcjogdC5wcmltYXJ5Q29sb3IgfX0+XG4vLyAgICAgICAgIEhlbGxvXG4vLyAgICAgICAgIDxUd2Vha3NQYW5lbD5cbi8vICAgICAgICAgICA8VHdlYWtTZWN0aW9uIGxhYmVsPVwiVHlwb2dyYXBoeVwiIC8+XG4vLyAgICAgICAgICAgPFR3ZWFrU2xpZGVyIGxhYmVsPVwiRm9udCBzaXplXCIgdmFsdWU9e3QuZm9udFNpemV9IG1pbj17MTB9IG1heD17MzJ9IHVuaXQ9XCJweFwiXG4vLyAgICAgICAgICAgICAgICAgICAgICAgIG9uQ2hhbmdlPXsodikgPT4gc2V0VHdlYWsoJ2ZvbnRTaXplJywgdil9IC8+XG4vLyAgICAgICAgICAgPFR3ZWFrUmFkaW8gIGxhYmVsPVwiRGVuc2l0eVwiIHZhbHVlPXt0LmRlbnNpdHl9XG4vLyAgICAgICAgICAgICAgICAgICAgICAgIG9wdGlvbnM9e1snY29tcGFjdCcsICdyZWd1bGFyJywgJ2NvbWZ5J119XG4vLyAgICAgICAgICAgICAgICAgICAgICAgIG9uQ2hhbmdlPXsodikgPT4gc2V0VHdlYWsoJ2RlbnNpdHknLCB2KX0gLz5cbi8vICAgICAgICAgICA8VHdlYWtTZWN0aW9uIGxhYmVsPVwiVGhlbWVcIiAvPlxuLy8gICAgICAgICAgIDxUd2Vha0NvbG9yICBsYWJlbD1cIlByaW1hcnlcIiB2YWx1ZT17dC5wcmltYXJ5Q29sb3J9XG4vLyAgICAgICAgICAgICAgICAgICAgICAgIG9wdGlvbnM9e1snI0Q5Nzc1NycsICcjMkE2RkRCJywgJyMxRjhBNUInLCAnIzdBNUFFMCddfVxuLy8gICAgICAgICAgICAgICAgICAgICAgICBvbkNoYW5nZT17KHYpID0+IHNldFR3ZWFrKCdwcmltYXJ5Q29sb3InLCB2KX0gLz5cbi8vICAgICAgICAgICA8VHdlYWtDb2xvciAgbGFiZWw9XCJQYWxldHRlXCIgdmFsdWU9e3QucGFsZXR0ZX1cbi8vICAgICAgICAgICAgICAgICAgICAgICAgb3B0aW9ucz17W1snI0Q5Nzc1NycsICcjMjkyNjFiJywgJyNmNmY0ZWYnXSxcbi8vICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgIFsnIzQ3NTU2OScsICcjMGYxNzJhJywgJyNmMWY1ZjknXV19XG4vLyAgICAgICAgICAgICAgICAgICAgICAgIG9uQ2hhbmdlPXsodikgPT4gc2V0VHdlYWsoJ3BhbGV0dGUnLCB2KX0gLz5cbi8vICAgICAgICAgICA8VHdlYWtUb2dnbGUgbGFiZWw9XCJEYXJrIG1vZGVcIiB2YWx1ZT17dC5kYXJrfVxuLy8gICAgICAgICAgICAgICAgICAgICAgICBvbkNoYW5nZT17KHYpID0+IHNldFR3ZWFrKCdkYXJrJywgdil9IC8+XG4vLyAgICAgICAgIDwvVHdlYWtzUGFuZWw+XG4vLyAgICAgICA8L2Rpdj5cbi8vICAgICApO1xuLy8gICB9XG4vL1xuLy8g4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSAXG5cbmNvbnN0IF9fVFdFQUtTX1NUWUxFID0gYFxuICAudHdrLXBhbmVse3Bvc2l0aW9uOmZpeGVkO3JpZ2h0OjE2cHg7Ym90dG9tOjE2cHg7ei1pbmRleDoyMTQ3NDgzNjQ2O3dpZHRoOjI4MHB4O1xuICAgIG1heC1oZWlnaHQ6Y2FsYygxMDB2aCAtIDMycHgpO2Rpc3BsYXk6ZmxleDtmbGV4LWRpcmVjdGlvbjpjb2x1bW47XG4gICAgdHJhbnNmb3JtOnNjYWxlKHZhcigtLWRjLWludi16b29tLDEpKTt0cmFuc2Zvcm0tb3JpZ2luOmJvdHRvbSByaWdodDtcbiAgICBiYWNrZ3JvdW5kOnJnYmEoMjUwLDI0OSwyNDcsLjc4KTtjb2xvcjojMjkyNjFiO1xuICAgIC13ZWJraXQtYmFja2Ryb3AtZmlsdGVyOmJsdXIoMjRweCkgc2F0dXJhdGUoMTYwJSk7YmFja2Ryb3AtZmlsdGVyOmJsdXIoMjRweCkgc2F0dXJhdGUoMTYwJSk7XG4gICAgYm9yZGVyOi41cHggc29saWQgcmdiYSgyNTUsMjU1LDI1NSwuNik7Ym9yZGVyLXJhZGl1czoxNHB4O1xuICAgIGJveC1zaGFkb3c6MCAxcHggMCByZ2JhKDI1NSwyNTUsMjU1LC41KSBpbnNldCwwIDEycHggNDBweCByZ2JhKDAsMCwwLC4xOCk7XG4gICAgZm9udDoxMS41cHgvMS40IHVpLXNhbnMtc2VyaWYsc3lzdGVtLXVpLC1hcHBsZS1zeXN0ZW0sc2Fucy1zZXJpZjtvdmVyZmxvdzpoaWRkZW59XG4gIC50d2staGR7ZGlzcGxheTpmbGV4O2FsaWduLWl0ZW1zOmNlbnRlcjtqdXN0aWZ5LWNvbnRlbnQ6c3BhY2UtYmV0d2VlbjtcbiAgICBwYWRkaW5nOjEwcHggOHB4IDEwcHggMTRweDtjdXJzb3I6bW92ZTt1c2VyLXNlbGVjdDpub25lfVxuICAudHdrLWhkIGJ7Zm9udC1zaXplOjEycHg7Zm9udC13ZWlnaHQ6NjAwO2xldHRlci1zcGFjaW5nOi4wMWVtfVxuICAudHdrLXh7YXBwZWFyYW5jZTpub25lO2JvcmRlcjowO2JhY2tncm91bmQ6dHJhbnNwYXJlbnQ7Y29sb3I6cmdiYSg0MSwzOCwyNywuNTUpO1xuICAgIHdpZHRoOjIycHg7aGVpZ2h0OjIycHg7Ym9yZGVyLXJhZGl1czo2cHg7Y3Vyc29yOmRlZmF1bHQ7Zm9udC1zaXplOjEzcHg7bGluZS1oZWlnaHQ6MX1cbiAgLnR3ay14OmhvdmVye2JhY2tncm91bmQ6cmdiYSgwLDAsMCwuMDYpO2NvbG9yOiMyOTI2MWJ9XG4gIC50d2stYm9keXtwYWRkaW5nOjJweCAxNHB4IDE0cHg7ZGlzcGxheTpmbGV4O2ZsZXgtZGlyZWN0aW9uOmNvbHVtbjtnYXA6MTBweDtcbiAgICBvdmVyZmxvdy15OmF1dG87b3ZlcmZsb3cteDpoaWRkZW47bWluLWhlaWdodDowO1xuICAgIHNjcm9sbGJhci13aWR0aDp0aGluO3Njcm9sbGJhci1jb2xvcjpyZ2JhKDAsMCwwLC4xNSkgdHJhbnNwYXJlbnR9XG4gIC50d2stYm9keTo6LXdlYmtpdC1zY3JvbGxiYXJ7d2lkdGg6OHB4fVxuICAudHdrLWJvZHk6Oi13ZWJraXQtc2Nyb2xsYmFyLXRyYWNre2JhY2tncm91bmQ6dHJhbnNwYXJlbnQ7bWFyZ2luOjJweH1cbiAgLnR3ay1ib2R5Ojotd2Via2l0LXNjcm9sbGJhci10aHVtYntiYWNrZ3JvdW5kOnJnYmEoMCwwLDAsLjE1KTtib3JkZXItcmFkaXVzOjRweDtcbiAgICBib3JkZXI6MnB4IHNvbGlkIHRyYW5zcGFyZW50O2JhY2tncm91bmQtY2xpcDpjb250ZW50LWJveH1cbiAgLnR3ay1ib2R5Ojotd2Via2l0LXNjcm9sbGJhci10aHVtYjpob3ZlcntiYWNrZ3JvdW5kOnJnYmEoMCwwLDAsLjI1KTtcbiAgICBib3JkZXI6MnB4IHNvbGlkIHRyYW5zcGFyZW50O2JhY2tncm91bmQtY2xpcDpjb250ZW50LWJveH1cbiAgLnR3ay1yb3d7ZGlzcGxheTpmbGV4O2ZsZXgtZGlyZWN0aW9uOmNvbHVtbjtnYXA6NXB4fVxuICAudHdrLXJvdy1oe2ZsZXgtZGlyZWN0aW9uOnJvdzthbGlnbi1pdGVtczpjZW50ZXI7anVzdGlmeS1jb250ZW50OnNwYWNlLWJldHdlZW47Z2FwOjEwcHh9XG4gIC50d2stbGJse2Rpc3BsYXk6ZmxleDtqdXN0aWZ5LWNvbnRlbnQ6c3BhY2UtYmV0d2VlbjthbGlnbi1pdGVtczpiYXNlbGluZTtcbiAgICBjb2xvcjpyZ2JhKDQxLDM4LDI3LC43Mil9XG4gIC50d2stbGJsPnNwYW46Zmlyc3QtY2hpbGR7Zm9udC13ZWlnaHQ6NTAwfVxuICAudHdrLXZhbHtjb2xvcjpyZ2JhKDQxLDM4LDI3LC41KTtmb250LXZhcmlhbnQtbnVtZXJpYzp0YWJ1bGFyLW51bXN9XG5cbiAgLnR3ay1zZWN0e2ZvbnQtc2l6ZToxMHB4O2ZvbnQtd2VpZ2h0OjYwMDtsZXR0ZXItc3BhY2luZzouMDZlbTt0ZXh0LXRyYW5zZm9ybTp1cHBlcmNhc2U7XG4gICAgY29sb3I6cmdiYSg0MSwzOCwyNywuNDUpO3BhZGRpbmc6MTBweCAwIDB9XG4gIC50d2stc2VjdDpmaXJzdC1jaGlsZHtwYWRkaW5nLXRvcDowfVxuXG4gIC50d2stZmllbGR7YXBwZWFyYW5jZTpub25lO2JveC1zaXppbmc6Ym9yZGVyLWJveDt3aWR0aDoxMDAlO21pbi13aWR0aDowO2hlaWdodDoyNnB4O3BhZGRpbmc6MCA4cHg7XG4gICAgYm9yZGVyOi41cHggc29saWQgcmdiYSgwLDAsMCwuMSk7Ym9yZGVyLXJhZGl1czo3cHg7XG4gICAgYmFja2dyb3VuZDpyZ2JhKDI1NSwyNTUsMjU1LC42KTtjb2xvcjppbmhlcml0O2ZvbnQ6aW5oZXJpdDtvdXRsaW5lOm5vbmV9XG4gIC50d2stZmllbGQ6Zm9jdXN7Ym9yZGVyLWNvbG9yOnJnYmEoMCwwLDAsLjI1KTtiYWNrZ3JvdW5kOnJnYmEoMjU1LDI1NSwyNTUsLjg1KX1cbiAgc2VsZWN0LnR3ay1maWVsZHtwYWRkaW5nLXJpZ2h0OjIycHg7XG4gICAgYmFja2dyb3VuZC1pbWFnZTp1cmwoXCJkYXRhOmltYWdlL3N2Zyt4bWw7dXRmOCw8c3ZnIHhtbG5zPSdodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2Zycgd2lkdGg9JzEwJyBoZWlnaHQ9JzYnIHZpZXdCb3g9JzAgMCAxMCA2Jz48cGF0aCBmaWxsPSdyZ2JhKDAsMCwwLC41KScgZD0nTTAgMGgxMEw1IDZ6Jy8+PC9zdmc+XCIpO1xuICAgIGJhY2tncm91bmQtcmVwZWF0Om5vLXJlcGVhdDtiYWNrZ3JvdW5kLXBvc2l0aW9uOnJpZ2h0IDhweCBjZW50ZXJ9XG5cbiAgLnR3ay1zbGlkZXJ7YXBwZWFyYW5jZTpub25lOy13ZWJraXQtYXBwZWFyYW5jZTpub25lO3dpZHRoOjEwMCU7aGVpZ2h0OjRweDttYXJnaW46NnB4IDA7XG4gICAgYm9yZGVyLXJhZGl1czo5OTlweDtiYWNrZ3JvdW5kOnJnYmEoMCwwLDAsLjEyKTtvdXRsaW5lOm5vbmV9XG4gIC50d2stc2xpZGVyOjotd2Via2l0LXNsaWRlci10aHVtYnstd2Via2l0LWFwcGVhcmFuY2U6bm9uZTthcHBlYXJhbmNlOm5vbmU7XG4gICAgd2lkdGg6MTRweDtoZWlnaHQ6MTRweDtib3JkZXItcmFkaXVzOjUwJTtiYWNrZ3JvdW5kOiNmZmY7XG4gICAgYm9yZGVyOi41cHggc29saWQgcmdiYSgwLDAsMCwuMTIpO2JveC1zaGFkb3c6MCAxcHggM3B4IHJnYmEoMCwwLDAsLjIpO2N1cnNvcjpkZWZhdWx0fVxuICAudHdrLXNsaWRlcjo6LW1vei1yYW5nZS10aHVtYnt3aWR0aDoxNHB4O2hlaWdodDoxNHB4O2JvcmRlci1yYWRpdXM6NTAlO1xuICAgIGJhY2tncm91bmQ6I2ZmZjtib3JkZXI6LjVweCBzb2xpZCByZ2JhKDAsMCwwLC4xMik7Ym94LXNoYWRvdzowIDFweCAzcHggcmdiYSgwLDAsMCwuMik7Y3Vyc29yOmRlZmF1bHR9XG5cbiAgLnR3ay1zZWd7cG9zaXRpb246cmVsYXRpdmU7ZGlzcGxheTpmbGV4O3BhZGRpbmc6MnB4O2JvcmRlci1yYWRpdXM6OHB4O1xuICAgIGJhY2tncm91bmQ6cmdiYSgwLDAsMCwuMDYpO3VzZXItc2VsZWN0Om5vbmV9XG4gIC50d2stc2VnLXRodW1ie3Bvc2l0aW9uOmFic29sdXRlO3RvcDoycHg7Ym90dG9tOjJweDtib3JkZXItcmFkaXVzOjZweDtcbiAgICBiYWNrZ3JvdW5kOnJnYmEoMjU1LDI1NSwyNTUsLjkpO2JveC1zaGFkb3c6MCAxcHggMnB4IHJnYmEoMCwwLDAsLjEyKTtcbiAgICB0cmFuc2l0aW9uOmxlZnQgLjE1cyBjdWJpYy1iZXppZXIoLjMsLjcsLjQsMSksd2lkdGggLjE1c31cbiAgLnR3ay1zZWcuZHJhZ2dpbmcgLnR3ay1zZWctdGh1bWJ7dHJhbnNpdGlvbjpub25lfVxuICAudHdrLXNlZyBidXR0b257YXBwZWFyYW5jZTpub25lO3Bvc2l0aW9uOnJlbGF0aXZlO3otaW5kZXg6MTtmbGV4OjE7Ym9yZGVyOjA7XG4gICAgYmFja2dyb3VuZDp0cmFuc3BhcmVudDtjb2xvcjppbmhlcml0O2ZvbnQ6aW5oZXJpdDtmb250LXdlaWdodDo1MDA7bWluLWhlaWdodDoyMnB4O1xuICAgIGJvcmRlci1yYWRpdXM6NnB4O2N1cnNvcjpkZWZhdWx0O3BhZGRpbmc6NHB4IDZweDtsaW5lLWhlaWdodDoxLjI7XG4gICAgb3ZlcmZsb3ctd3JhcDphbnl3aGVyZX1cblxuICAudHdrLXRvZ2dsZXtwb3NpdGlvbjpyZWxhdGl2ZTt3aWR0aDozMnB4O2hlaWdodDoxOHB4O2JvcmRlcjowO2JvcmRlci1yYWRpdXM6OTk5cHg7XG4gICAgYmFja2dyb3VuZDpyZ2JhKDAsMCwwLC4xNSk7dHJhbnNpdGlvbjpiYWNrZ3JvdW5kIC4xNXM7Y3Vyc29yOmRlZmF1bHQ7cGFkZGluZzowfVxuICAudHdrLXRvZ2dsZVtkYXRhLW9uPVwiMVwiXXtiYWNrZ3JvdW5kOiMzNGM3NTl9XG4gIC50d2stdG9nZ2xlIGl7cG9zaXRpb246YWJzb2x1dGU7dG9wOjJweDtsZWZ0OjJweDt3aWR0aDoxNHB4O2hlaWdodDoxNHB4O2JvcmRlci1yYWRpdXM6NTAlO1xuICAgIGJhY2tncm91bmQ6I2ZmZjtib3gtc2hhZG93OjAgMXB4IDJweCByZ2JhKDAsMCwwLC4yNSk7dHJhbnNpdGlvbjp0cmFuc2Zvcm0gLjE1c31cbiAgLnR3ay10b2dnbGVbZGF0YS1vbj1cIjFcIl0gaXt0cmFuc2Zvcm06dHJhbnNsYXRlWCgxNHB4KX1cblxuICAudHdrLW51bXtkaXNwbGF5OmZsZXg7YWxpZ24taXRlbXM6Y2VudGVyO2JveC1zaXppbmc6Ym9yZGVyLWJveDttaW4td2lkdGg6MDtoZWlnaHQ6MjZweDtwYWRkaW5nOjAgMCAwIDhweDtcbiAgICBib3JkZXI6LjVweCBzb2xpZCByZ2JhKDAsMCwwLC4xKTtib3JkZXItcmFkaXVzOjdweDtiYWNrZ3JvdW5kOnJnYmEoMjU1LDI1NSwyNTUsLjYpfVxuICAudHdrLW51bS1sYmx7Zm9udC13ZWlnaHQ6NTAwO2NvbG9yOnJnYmEoNDEsMzgsMjcsLjYpO2N1cnNvcjpldy1yZXNpemU7XG4gICAgdXNlci1zZWxlY3Q6bm9uZTtwYWRkaW5nLXJpZ2h0OjhweH1cbiAgLnR3ay1udW0gaW5wdXR7ZmxleDoxO21pbi13aWR0aDowO2hlaWdodDoxMDAlO2JvcmRlcjowO2JhY2tncm91bmQ6dHJhbnNwYXJlbnQ7XG4gICAgZm9udDppbmhlcml0O2ZvbnQtdmFyaWFudC1udW1lcmljOnRhYnVsYXItbnVtczt0ZXh0LWFsaWduOnJpZ2h0O3BhZGRpbmc6MCA4cHggMCAwO1xuICAgIG91dGxpbmU6bm9uZTtjb2xvcjppbmhlcml0Oy1tb3otYXBwZWFyYW5jZTp0ZXh0ZmllbGR9XG4gIC50d2stbnVtIGlucHV0Ojotd2Via2l0LWlubmVyLXNwaW4tYnV0dG9uLC50d2stbnVtIGlucHV0Ojotd2Via2l0LW91dGVyLXNwaW4tYnV0dG9ue1xuICAgIC13ZWJraXQtYXBwZWFyYW5jZTpub25lO21hcmdpbjowfVxuICAudHdrLW51bS11bml0e3BhZGRpbmctcmlnaHQ6OHB4O2NvbG9yOnJnYmEoNDEsMzgsMjcsLjQ1KX1cblxuICAudHdrLWJ0bnthcHBlYXJhbmNlOm5vbmU7aGVpZ2h0OjI2cHg7cGFkZGluZzowIDEycHg7Ym9yZGVyOjA7Ym9yZGVyLXJhZGl1czo3cHg7XG4gICAgYmFja2dyb3VuZDpyZ2JhKDAsMCwwLC43OCk7Y29sb3I6I2ZmZjtmb250OmluaGVyaXQ7Zm9udC13ZWlnaHQ6NTAwO2N1cnNvcjpkZWZhdWx0fVxuICAudHdrLWJ0bjpob3ZlcntiYWNrZ3JvdW5kOnJnYmEoMCwwLDAsLjg4KX1cbiAgLnR3ay1idG4uc2Vjb25kYXJ5e2JhY2tncm91bmQ6cmdiYSgwLDAsMCwuMDYpO2NvbG9yOmluaGVyaXR9XG4gIC50d2stYnRuLnNlY29uZGFyeTpob3ZlcntiYWNrZ3JvdW5kOnJnYmEoMCwwLDAsLjEpfVxuXG4gIC50d2stc3dhdGNoe2FwcGVhcmFuY2U6bm9uZTstd2Via2l0LWFwcGVhcmFuY2U6bm9uZTt3aWR0aDo1NnB4O2hlaWdodDoyMnB4O1xuICAgIGJvcmRlcjouNXB4IHNvbGlkIHJnYmEoMCwwLDAsLjEpO2JvcmRlci1yYWRpdXM6NnB4O3BhZGRpbmc6MDtjdXJzb3I6ZGVmYXVsdDtcbiAgICBiYWNrZ3JvdW5kOnRyYW5zcGFyZW50O2ZsZXgtc2hyaW5rOjB9XG4gIC50d2stc3dhdGNoOjotd2Via2l0LWNvbG9yLXN3YXRjaC13cmFwcGVye3BhZGRpbmc6MH1cbiAgLnR3ay1zd2F0Y2g6Oi13ZWJraXQtY29sb3Itc3dhdGNoe2JvcmRlcjowO2JvcmRlci1yYWRpdXM6NS41cHh9XG4gIC50d2stc3dhdGNoOjotbW96LWNvbG9yLXN3YXRjaHtib3JkZXI6MDtib3JkZXItcmFkaXVzOjUuNXB4fVxuXG4gIC50d2stY2hpcHN7ZGlzcGxheTpmbGV4O2dhcDo2cHh9XG4gIC50d2stY2hpcHtwb3NpdGlvbjpyZWxhdGl2ZTthcHBlYXJhbmNlOm5vbmU7ZmxleDoxO21pbi13aWR0aDowO2hlaWdodDo0NnB4O1xuICAgIHBhZGRpbmc6MDtib3JkZXI6MDtib3JkZXItcmFkaXVzOjZweDtvdmVyZmxvdzpoaWRkZW47Y3Vyc29yOmRlZmF1bHQ7XG4gICAgYm94LXNoYWRvdzowIDAgMCAuNXB4IHJnYmEoMCwwLDAsLjEyKSwwIDFweCAycHggcmdiYSgwLDAsMCwuMDYpO1xuICAgIHRyYW5zaXRpb246dHJhbnNmb3JtIC4xMnMgY3ViaWMtYmV6aWVyKC4zLC43LC40LDEpLGJveC1zaGFkb3cgLjEyc31cbiAgLnR3ay1jaGlwOmhvdmVye3RyYW5zZm9ybTp0cmFuc2xhdGVZKC0xcHgpO1xuICAgIGJveC1zaGFkb3c6MCAwIDAgLjVweCByZ2JhKDAsMCwwLC4xOCksMCA0cHggMTBweCByZ2JhKDAsMCwwLC4xMil9XG4gIC50d2stY2hpcFtkYXRhLW9uPVwiMVwiXXtib3gtc2hhZG93OjAgMCAwIDEuNXB4IHJnYmEoMCwwLDAsLjg1KSxcbiAgICAwIDJweCA2cHggcmdiYSgwLDAsMCwuMTUpfVxuICAudHdrLWNoaXA+c3Bhbntwb3NpdGlvbjphYnNvbHV0ZTt0b3A6MDtib3R0b206MDtyaWdodDowO3dpZHRoOjM0JTtcbiAgICBkaXNwbGF5OmZsZXg7ZmxleC1kaXJlY3Rpb246Y29sdW1uO2JveC1zaGFkb3c6LTFweCAwIDAgcmdiYSgwLDAsMCwuMSl9XG4gIC50d2stY2hpcD5zcGFuPml7ZmxleDoxO2JveC1zaGFkb3c6MCAtMXB4IDAgcmdiYSgwLDAsMCwuMSl9XG4gIC50d2stY2hpcD5zcGFuPmk6Zmlyc3QtY2hpbGR7Ym94LXNoYWRvdzpub25lfVxuICAudHdrLWNoaXAgc3Zne3Bvc2l0aW9uOmFic29sdXRlO3RvcDo2cHg7bGVmdDo2cHg7d2lkdGg6MTNweDtoZWlnaHQ6MTNweDtcbiAgICBmaWx0ZXI6ZHJvcC1zaGFkb3coMCAxcHggMXB4IHJnYmEoMCwwLDAsLjMpKX1cbmA7XG5cbi8vIOKUgOKUgCB1c2VUd2Vha3Mg4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSA4pSAXG4vLyBTaW5nbGUgc291cmNlIG9mIHRydXRoIGZvciB0d2VhayB2YWx1ZXMuIHNldFR3ZWFrIHBlcnNpc3RzIHZpYSB0aGUgaG9zdFxuLy8gKF9fZWRpdF9tb2RlX3NldF9rZXlzIOKGkiBob3N0IHJld3JpdGVzIHRoZSBFRElUTU9ERSBibG9jayBvbiBkaXNrKS5cbmZ1bmN0aW9uIHVzZVR3ZWFrcyhkZWZhdWx0cykge1xuICBjb25zdCBbdmFsdWVzLCBzZXRWYWx1ZXNdID0gUmVhY3QudXNlU3RhdGUoZGVmYXVsdHMpO1xuICAvLyBBY2NlcHRzIGVpdGhlciBzZXRUd2Vhaygna2V5JywgdmFsdWUpIG9yIHNldFR3ZWFrKHsga2V5OiB2YWx1ZSwgLi4uIH0pIHNvIGFcbiAgLy8gdXNlU3RhdGUtc3R5bGUgY2FsbCBkb2Vzbid0IHdyaXRlIGEgXCJbb2JqZWN0IE9iamVjdF1cIiBrZXkgaW50byB0aGUgcGVyc2lzdGVkXG4gIC8vIEpTT04gYmxvY2suXG4gIGNvbnN0IHNldFR3ZWFrID0gUmVhY3QudXNlQ2FsbGJhY2soKGtleU9yRWRpdHMsIHZhbCkgPT4ge1xuICAgIGNvbnN0IGVkaXRzID0gdHlwZW9mIGtleU9yRWRpdHMgPT09ICdvYmplY3QnICYmIGtleU9yRWRpdHMgIT09IG51bGxcbiAgICAgID8ga2V5T3JFZGl0cyA6IHsgW2tleU9yRWRpdHNdOiB2YWwgfTtcbiAgICBzZXRWYWx1ZXMoKHByZXYpID0+ICh7IC4uLnByZXYsIC4uLmVkaXRzIH0pKTtcbiAgICB3aW5kb3cucGFyZW50LnBvc3RNZXNzYWdlKHsgdHlwZTogJ19fZWRpdF9tb2RlX3NldF9rZXlzJywgZWRpdHMgfSwgJyonKTtcbiAgICAvLyBTYW1lLXdpbmRvdyBzaWduYWwgc28gaW4tcGFnZSBsaXN0ZW5lcnMgKGRlY2stc3RhZ2UgcmFpbCB0aHVtYm5haWxzKVxuICAgIC8vIGNhbiByZWFjdCDigJQgdGhlIHBhcmVudCBtZXNzYWdlIG9ubHkgcmVhY2hlcyB0aGUgaG9zdCwgbm90IHBlZXJzLlxuICAgIHdpbmRvdy5kaXNwYXRjaEV2ZW50KG5ldyBDdXN0b21FdmVudCgndHdlYWtjaGFuZ2UnLCB7IGRldGFpbDogZWRpdHMgfSkpO1xuICB9LCBbXSk7XG4gIHJldHVybiBbdmFsdWVzLCBzZXRUd2Vha107XG59XG5cbi8vIOKUgOKUgCBUd2Vha3NQYW5lbCDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIBcbi8vIEZsb2F0aW5nIHNoZWxsLiBSZWdpc3RlcnMgdGhlIHByb3RvY29sIGxpc3RlbmVyIEJFRk9SRSBhbm5vdW5jaW5nXG4vLyBhdmFpbGFiaWxpdHkg4oCUIGlmIHRoZSBhbm5vdW5jZSByYW4gZmlyc3QsIHRoZSBob3N0J3MgYWN0aXZhdGUgY291bGQgbGFuZFxuLy8gYmVmb3JlIG91ciBoYW5kbGVyIGV4aXN0cyBhbmQgdGhlIHRvb2xiYXIgdG9nZ2xlIHdvdWxkIHNpbGVudGx5IG5vLW9wLlxuLy8gVGhlIGNsb3NlIGJ1dHRvbiBwb3N0cyBfX2VkaXRfbW9kZV9kaXNtaXNzZWQgc28gdGhlIGhvc3QncyB0b29sYmFyIHRvZ2dsZVxuLy8gZmxpcHMgb2ZmIGluIGxvY2tzdGVwOyB0aGUgaG9zdCBlY2hvZXMgX19kZWFjdGl2YXRlX2VkaXRfbW9kZSBiYWNrIHdoaWNoXG4vLyBpcyB3aGF0IGFjdHVhbGx5IGhpZGVzIHRoZSBwYW5lbC5cbmZ1bmN0aW9uIFR3ZWFrc1BhbmVsKHsgdGl0bGUgPSAnVHdlYWtzJywgbm9EZWNrQ29udHJvbHMgPSBmYWxzZSwgY2hpbGRyZW4gfSkge1xuICBjb25zdCBbb3Blbiwgc2V0T3Blbl0gPSBSZWFjdC51c2VTdGF0ZShmYWxzZSk7XG4gIGNvbnN0IGRyYWdSZWYgPSBSZWFjdC51c2VSZWYobnVsbCk7XG4gIC8vIEF1dG8taW5qZWN0IGEgcmFpbCB0b2dnbGUgd2hlbiBhIDxkZWNrLXN0YWdlPiBpcyBvbiB0aGUgcGFnZS4gVGhlXG4gIC8vIHRvZ2dsZSBkcml2ZXMgdGhlIGRlY2sncyBwZXItdmlld2VyIF9yYWlsVmlzaWJsZSB2aWEgd2luZG93IG1lc3NhZ2U7XG4gIC8vIHN0YXRlIGlzIG1pcnJvcmVkIGZyb20gdGhlIHNhbWUgbG9jYWxTdG9yYWdlIGtleSB0aGUgZGVjayByZWFkcyBzb1xuICAvLyB0aGUgY29udHJvbCByZWZsZWN0cyByZWFsaXR5IGFjcm9zcyByZWxvYWRzLiBUaGUgbWVjaGFuaXNtIGlzIHRoZVxuICAvLyBtZXNzYWdlIOKAlCBhdXRob3JzIHdobyB3YW50IGN1c3RvbSBwbGFjZW1lbnQgY2FuIHBvc3QgaXQgZGlyZWN0bHlcbiAgLy8gYW5kIHBhc3Mgbm9EZWNrQ29udHJvbHMgdG8gc3VwcHJlc3MgdGhpcyBvbmUuXG4gIGNvbnN0IGhhc0RlY2tTdGFnZSA9IFJlYWN0LnVzZU1lbW8oXG4gICAgKCkgPT4gdHlwZW9mIGRvY3VtZW50ICE9PSAndW5kZWZpbmVkJyAmJiAhIWRvY3VtZW50LnF1ZXJ5U2VsZWN0b3IoJ2RlY2stc3RhZ2UnKSxcbiAgICBbXSxcbiAgKTtcbiAgLy8gZGVjay1zdGFnZSBlbmFibGVzIGl0cyByYWlsIGluIGNvbm5lY3RlZENhbGxiYWNrLCBidXQgdGhpcyBwYW5lbCBjYW5cbiAgLy8gbW91bnQgYmVmb3JlIHRoYXQgZWxlbWVudCBoYXMgdXBncmFkZWQuIFRoZSBpbml0aWFsIHJlYWQgY2F0Y2hlcyB0aGVcbiAgLy8gY29tbW9uIGNhc2U7IHRoZSBsaXN0ZW5lciBjb3ZlcnMgbW91bnRpbmcgZmlyc3QuIChPbGRlciBkZWNrLXN0YWdlLmpzXG4gIC8vIGNvcGllcyBzdGlsbCB3YWl0IGZvciB0aGUgaG9zdCdzIF9fb21lbGV0dGVfcmFpbF9lbmFibGVkIHBvc3RNZXNzYWdlIOKAlFxuICAvLyBzYW1lIGxpc3RlbmVyIGhhbmRsZXMgdGhvc2UuKVxuICBjb25zdCBbcmFpbEVuYWJsZWQsIHNldFJhaWxFbmFibGVkXSA9IFJlYWN0LnVzZVN0YXRlKFxuICAgICgpID0+IGhhc0RlY2tTdGFnZSAmJiAhIWRvY3VtZW50LnF1ZXJ5U2VsZWN0b3IoJ2RlY2stc3RhZ2UnKT8uX3JhaWxFbmFibGVkLFxuICApO1xuICBSZWFjdC51c2VFZmZlY3QoKCkgPT4ge1xuICAgIGlmICghaGFzRGVja1N0YWdlIHx8IHJhaWxFbmFibGVkKSByZXR1cm4gdW5kZWZpbmVkO1xuICAgIGNvbnN0IG9uTXNnID0gKGUpID0+IHtcbiAgICAgIGlmIChlLmRhdGEgJiYgZS5kYXRhLnR5cGUgPT09ICdfX29tZWxldHRlX3JhaWxfZW5hYmxlZCcpIHNldFJhaWxFbmFibGVkKHRydWUpO1xuICAgIH07XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ21lc3NhZ2UnLCBvbk1zZyk7XG4gICAgcmV0dXJuICgpID0+IHdpbmRvdy5yZW1vdmVFdmVudExpc3RlbmVyKCdtZXNzYWdlJywgb25Nc2cpO1xuICB9LCBbaGFzRGVja1N0YWdlLCByYWlsRW5hYmxlZF0pO1xuICBjb25zdCBbcmFpbFZpc2libGUsIHNldFJhaWxWaXNpYmxlXSA9IFJlYWN0LnVzZVN0YXRlKCgpID0+IHtcbiAgICB0cnkgeyByZXR1cm4gbG9jYWxTdG9yYWdlLmdldEl0ZW0oJ2RlY2stc3RhZ2UucmFpbFZpc2libGUnKSAhPT0gJzAnOyB9IGNhdGNoIChlKSB7IHJldHVybiB0cnVlOyB9XG4gIH0pO1xuICBjb25zdCB0b2dnbGVSYWlsID0gKG9uKSA9PiB7XG4gICAgc2V0UmFpbFZpc2libGUob24pO1xuICAgIHdpbmRvdy5wb3N0TWVzc2FnZSh7IHR5cGU6ICdfX2RlY2tfcmFpbF92aXNpYmxlJywgb24gfSwgJyonKTtcbiAgfTtcbiAgY29uc3Qgb2Zmc2V0UmVmID0gUmVhY3QudXNlUmVmKHsgeDogMTYsIHk6IDE2IH0pO1xuICBjb25zdCBQQUQgPSAxNjtcblxuICBjb25zdCBjbGFtcFRvVmlld3BvcnQgPSBSZWFjdC51c2VDYWxsYmFjaygoKSA9PiB7XG4gICAgY29uc3QgcGFuZWwgPSBkcmFnUmVmLmN1cnJlbnQ7XG4gICAgaWYgKCFwYW5lbCkgcmV0dXJuO1xuICAgIGNvbnN0IHcgPSBwYW5lbC5vZmZzZXRXaWR0aCwgaCA9IHBhbmVsLm9mZnNldEhlaWdodDtcbiAgICBjb25zdCBtYXhSaWdodCA9IE1hdGgubWF4KFBBRCwgd2luZG93LmlubmVyV2lkdGggLSB3IC0gUEFEKTtcbiAgICBjb25zdCBtYXhCb3R0b20gPSBNYXRoLm1heChQQUQsIHdpbmRvdy5pbm5lckhlaWdodCAtIGggLSBQQUQpO1xuICAgIG9mZnNldFJlZi5jdXJyZW50ID0ge1xuICAgICAgeDogTWF0aC5taW4obWF4UmlnaHQsIE1hdGgubWF4KFBBRCwgb2Zmc2V0UmVmLmN1cnJlbnQueCkpLFxuICAgICAgeTogTWF0aC5taW4obWF4Qm90dG9tLCBNYXRoLm1heChQQUQsIG9mZnNldFJlZi5jdXJyZW50LnkpKSxcbiAgICB9O1xuICAgIHBhbmVsLnN0eWxlLnJpZ2h0ID0gb2Zmc2V0UmVmLmN1cnJlbnQueCArICdweCc7XG4gICAgcGFuZWwuc3R5bGUuYm90dG9tID0gb2Zmc2V0UmVmLmN1cnJlbnQueSArICdweCc7XG4gIH0sIFtdKTtcblxuICBSZWFjdC51c2VFZmZlY3QoKCkgPT4ge1xuICAgIGlmICghb3BlbikgcmV0dXJuO1xuICAgIGNsYW1wVG9WaWV3cG9ydCgpO1xuICAgIGlmICh0eXBlb2YgUmVzaXplT2JzZXJ2ZXIgPT09ICd1bmRlZmluZWQnKSB7XG4gICAgICB3aW5kb3cuYWRkRXZlbnRMaXN0ZW5lcigncmVzaXplJywgY2xhbXBUb1ZpZXdwb3J0KTtcbiAgICAgIHJldHVybiAoKSA9PiB3aW5kb3cucmVtb3ZlRXZlbnRMaXN0ZW5lcigncmVzaXplJywgY2xhbXBUb1ZpZXdwb3J0KTtcbiAgICB9XG4gICAgY29uc3Qgcm8gPSBuZXcgUmVzaXplT2JzZXJ2ZXIoY2xhbXBUb1ZpZXdwb3J0KTtcbiAgICByby5vYnNlcnZlKGRvY3VtZW50LmRvY3VtZW50RWxlbWVudCk7XG4gICAgcmV0dXJuICgpID0+IHJvLmRpc2Nvbm5lY3QoKTtcbiAgfSwgW29wZW4sIGNsYW1wVG9WaWV3cG9ydF0pO1xuXG4gIFJlYWN0LnVzZUVmZmVjdCgoKSA9PiB7XG4gICAgY29uc3Qgb25Nc2cgPSAoZSkgPT4ge1xuICAgICAgY29uc3QgdCA9IGU/LmRhdGE/LnR5cGU7XG4gICAgICBpZiAodCA9PT0gJ19fYWN0aXZhdGVfZWRpdF9tb2RlJykgc2V0T3Blbih0cnVlKTtcbiAgICAgIGVsc2UgaWYgKHQgPT09ICdfX2RlYWN0aXZhdGVfZWRpdF9tb2RlJykgc2V0T3BlbihmYWxzZSk7XG4gICAgfTtcbiAgICB3aW5kb3cuYWRkRXZlbnRMaXN0ZW5lcignbWVzc2FnZScsIG9uTXNnKTtcbiAgICB3aW5kb3cucGFyZW50LnBvc3RNZXNzYWdlKHsgdHlwZTogJ19fZWRpdF9tb2RlX2F2YWlsYWJsZScgfSwgJyonKTtcbiAgICByZXR1cm4gKCkgPT4gd2luZG93LnJlbW92ZUV2ZW50TGlzdGVuZXIoJ21lc3NhZ2UnLCBvbk1zZyk7XG4gIH0sIFtdKTtcblxuICBjb25zdCBkaXNtaXNzID0gKCkgPT4ge1xuICAgIHNldE9wZW4oZmFsc2UpO1xuICAgIHdpbmRvdy5wYXJlbnQucG9zdE1lc3NhZ2UoeyB0eXBlOiAnX19lZGl0X21vZGVfZGlzbWlzc2VkJyB9LCAnKicpO1xuICB9O1xuXG4gIGNvbnN0IG9uRHJhZ1N0YXJ0ID0gKGUpID0+IHtcbiAgICBjb25zdCBwYW5lbCA9IGRyYWdSZWYuY3VycmVudDtcbiAgICBpZiAoIXBhbmVsKSByZXR1cm47XG4gICAgY29uc3QgciA9IHBhbmVsLmdldEJvdW5kaW5nQ2xpZW50UmVjdCgpO1xuICAgIGNvbnN0IHN4ID0gZS5jbGllbnRYLCBzeSA9IGUuY2xpZW50WTtcbiAgICBjb25zdCBzdGFydFJpZ2h0ID0gd2luZG93LmlubmVyV2lkdGggLSByLnJpZ2h0O1xuICAgIGNvbnN0IHN0YXJ0Qm90dG9tID0gd2luZG93LmlubmVySGVpZ2h0IC0gci5ib3R0b207XG4gICAgY29uc3QgbW92ZSA9IChldikgPT4ge1xuICAgICAgb2Zmc2V0UmVmLmN1cnJlbnQgPSB7XG4gICAgICAgIHg6IHN0YXJ0UmlnaHQgLSAoZXYuY2xpZW50WCAtIHN4KSxcbiAgICAgICAgeTogc3RhcnRCb3R0b20gLSAoZXYuY2xpZW50WSAtIHN5KSxcbiAgICAgIH07XG4gICAgICBjbGFtcFRvVmlld3BvcnQoKTtcbiAgICB9O1xuICAgIGNvbnN0IHVwID0gKCkgPT4ge1xuICAgICAgd2luZG93LnJlbW92ZUV2ZW50TGlzdGVuZXIoJ21vdXNlbW92ZScsIG1vdmUpO1xuICAgICAgd2luZG93LnJlbW92ZUV2ZW50TGlzdGVuZXIoJ21vdXNldXAnLCB1cCk7XG4gICAgfTtcbiAgICB3aW5kb3cuYWRkRXZlbnRMaXN0ZW5lcignbW91c2Vtb3ZlJywgbW92ZSk7XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ21vdXNldXAnLCB1cCk7XG4gIH07XG5cbiAgaWYgKCFvcGVuKSByZXR1cm4gbnVsbDtcbiAgcmV0dXJuIChcbiAgICA8PlxuICAgICAgPHN0eWxlPntfX1RXRUFLU19TVFlMRX08L3N0eWxlPlxuICAgICAgPGRpdiByZWY9e2RyYWdSZWZ9IGNsYXNzTmFtZT1cInR3ay1wYW5lbFwiIGRhdGEtbm9uY29tbWVudGFibGU9XCJcIlxuICAgICAgICAgICBzdHlsZT17eyByaWdodDogb2Zmc2V0UmVmLmN1cnJlbnQueCwgYm90dG9tOiBvZmZzZXRSZWYuY3VycmVudC55IH19PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1oZFwiIG9uTW91c2VEb3duPXtvbkRyYWdTdGFydH0+XG4gICAgICAgICAgPGI+e3RpdGxlfTwvYj5cbiAgICAgICAgICA8YnV0dG9uIGNsYXNzTmFtZT1cInR3ay14XCIgYXJpYS1sYWJlbD1cIkNsb3NlIHR3ZWFrc1wiXG4gICAgICAgICAgICAgICAgICBvbk1vdXNlRG93bj17KGUpID0+IGUuc3RvcFByb3BhZ2F0aW9uKCl9XG4gICAgICAgICAgICAgICAgICBvbkNsaWNrPXtkaXNtaXNzfT7inJU8L2J1dHRvbj5cbiAgICAgICAgPC9kaXY+XG4gICAgICAgIDxkaXYgY2xhc3NOYW1lPVwidHdrLWJvZHlcIj5cbiAgICAgICAgICB7Y2hpbGRyZW59XG4gICAgICAgICAge2hhc0RlY2tTdGFnZSAmJiByYWlsRW5hYmxlZCAmJiAhbm9EZWNrQ29udHJvbHMgJiYgKFxuICAgICAgICAgICAgPFR3ZWFrU2VjdGlvbiBsYWJlbD1cIkRlY2tcIj5cbiAgICAgICAgICAgICAgPFR3ZWFrVG9nZ2xlIGxhYmVsPVwiVGh1bWJuYWlsIHJhaWxcIiB2YWx1ZT17cmFpbFZpc2libGV9IG9uQ2hhbmdlPXt0b2dnbGVSYWlsfSAvPlxuICAgICAgICAgICAgPC9Ud2Vha1NlY3Rpb24+XG4gICAgICAgICAgKX1cbiAgICAgICAgPC9kaXY+XG4gICAgICA8L2Rpdj5cbiAgICA8Lz5cbiAgKTtcbn1cblxuLy8g4pSA4pSAIExheW91dCBoZWxwZXJzIOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgOKUgFxuXG5mdW5jdGlvbiBUd2Vha1NlY3Rpb24oeyBsYWJlbCwgY2hpbGRyZW4gfSkge1xuICByZXR1cm4gKFxuICAgIDw+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1zZWN0XCI+e2xhYmVsfTwvZGl2PlxuICAgICAge2NoaWxkcmVufVxuICAgIDwvPlxuICApO1xufVxuXG5mdW5jdGlvbiBUd2Vha1Jvdyh7IGxhYmVsLCB2YWx1ZSwgY2hpbGRyZW4sIGlubGluZSA9IGZhbHNlIH0pIHtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT17aW5saW5lID8gJ3R3ay1yb3cgdHdrLXJvdy1oJyA6ICd0d2stcm93J30+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1sYmxcIj5cbiAgICAgICAgPHNwYW4+e2xhYmVsfTwvc3Bhbj5cbiAgICAgICAge3ZhbHVlICE9IG51bGwgJiYgPHNwYW4gY2xhc3NOYW1lPVwidHdrLXZhbFwiPnt2YWx1ZX08L3NwYW4+fVxuICAgICAgPC9kaXY+XG4gICAgICB7Y2hpbGRyZW59XG4gICAgPC9kaXY+XG4gICk7XG59XG5cbi8vIOKUgOKUgCBDb250cm9scyDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIDilIBcblxuZnVuY3Rpb24gVHdlYWtTbGlkZXIoeyBsYWJlbCwgdmFsdWUsIG1pbiA9IDAsIG1heCA9IDEwMCwgc3RlcCA9IDEsIHVuaXQgPSAnJywgb25DaGFuZ2UgfSkge1xuICByZXR1cm4gKFxuICAgIDxUd2Vha1JvdyBsYWJlbD17bGFiZWx9IHZhbHVlPXtgJHt2YWx1ZX0ke3VuaXR9YH0+XG4gICAgICA8aW5wdXQgdHlwZT1cInJhbmdlXCIgY2xhc3NOYW1lPVwidHdrLXNsaWRlclwiIG1pbj17bWlufSBtYXg9e21heH0gc3RlcD17c3RlcH1cbiAgICAgICAgICAgICB2YWx1ZT17dmFsdWV9IG9uQ2hhbmdlPXsoZSkgPT4gb25DaGFuZ2UoTnVtYmVyKGUudGFyZ2V0LnZhbHVlKSl9IC8+XG4gICAgPC9Ud2Vha1Jvdz5cbiAgKTtcbn1cblxuZnVuY3Rpb24gVHdlYWtUb2dnbGUoeyBsYWJlbCwgdmFsdWUsIG9uQ2hhbmdlIH0pIHtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1yb3cgdHdrLXJvdy1oXCI+XG4gICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1sYmxcIj48c3Bhbj57bGFiZWx9PC9zcGFuPjwvZGl2PlxuICAgICAgPGJ1dHRvbiB0eXBlPVwiYnV0dG9uXCIgY2xhc3NOYW1lPVwidHdrLXRvZ2dsZVwiIGRhdGEtb249e3ZhbHVlID8gJzEnIDogJzAnfVxuICAgICAgICAgICAgICByb2xlPVwic3dpdGNoXCIgYXJpYS1jaGVja2VkPXshIXZhbHVlfVxuICAgICAgICAgICAgICBvbkNsaWNrPXsoKSA9PiBvbkNoYW5nZSghdmFsdWUpfT48aSAvPjwvYnV0dG9uPlxuICAgIDwvZGl2PlxuICApO1xufVxuXG5mdW5jdGlvbiBUd2Vha1JhZGlvKHsgbGFiZWwsIHZhbHVlLCBvcHRpb25zLCBvbkNoYW5nZSB9KSB7XG4gIGNvbnN0IHRyYWNrUmVmID0gUmVhY3QudXNlUmVmKG51bGwpO1xuICBjb25zdCBbZHJhZ2dpbmcsIHNldERyYWdnaW5nXSA9IFJlYWN0LnVzZVN0YXRlKGZhbHNlKTtcbiAgLy8gVGhlIGFjdGl2ZSB2YWx1ZSBpcyByZWFkIGJ5IHBvaW50ZXItbW92ZSBoYW5kbGVycyBhdHRhY2hlZCBmb3IgdGhlIGxpZmV0aW1lXG4gIC8vIG9mIGEgZHJhZyDigJQgcmVmIGl0IHNvIGEgc3RhbGUgY2xvc3VyZSBkb2Vzbid0IGZpcmUgb25DaGFuZ2UgZm9yIGV2ZXJ5IG1vdmUuXG4gIGNvbnN0IHZhbHVlUmVmID0gUmVhY3QudXNlUmVmKHZhbHVlKTtcbiAgdmFsdWVSZWYuY3VycmVudCA9IHZhbHVlO1xuXG4gIC8vIFNlZ21lbnRzIHdyYXAgbWlkLXdvcmQgb25jZSBwZXItc2VnbWVudCB3aWR0aCBydW5zIG91dC4gVGhlIHRyYWNrIGlzXG4gIC8vIH4yNDhweCAoMjgwIHBhbmVsIOKIkiAyOCBib2R5IHBhZCDiiJIgNCBzZWcgcGFkKSwgZWFjaCBidXR0b24gbG9zZXMgMTJweFxuICAvLyB0byBpdHMgb3duIHBhZGRpbmcsIGFuZCAxMS41cHggc3lzdGVtLXVpIGF2ZXJhZ2VzIH42LjNweC9jaGFyIOKAlCBzbyAyXG4gIC8vIG9wdGlvbnMgZml0IH4xNiBjaGFycyBlYWNoLCAzIGZpdCB+MTAuIFBhc3QgdGhhdCAob3IgPjMgb3B0aW9ucyksIGZhbGxcbiAgLy8gYmFjayB0byBhIGRyb3Bkb3duIHJhdGhlciB0aGFuIHdyYXAuXG4gIGNvbnN0IGxhYmVsTGVuID0gKG8pID0+IFN0cmluZyh0eXBlb2YgbyA9PT0gJ29iamVjdCcgPyBvLmxhYmVsIDogbykubGVuZ3RoO1xuICBjb25zdCBtYXhMZW4gPSBvcHRpb25zLnJlZHVjZSgobSwgbykgPT4gTWF0aC5tYXgobSwgbGFiZWxMZW4obykpLCAwKTtcbiAgY29uc3QgZml0c0FzU2VnbWVudHMgPSBtYXhMZW4gPD0gKHsgMjogMTYsIDM6IDEwIH1bb3B0aW9ucy5sZW5ndGhdID8/IDApO1xuICBpZiAoIWZpdHNBc1NlZ21lbnRzKSB7XG4gICAgLy8gPHNlbGVjdD4gZW1pdHMgc3RyaW5ncyDigJQgbWFwIGJhY2sgdG8gdGhlIG9yaWdpbmFsIG9wdGlvbiB2YWx1ZSBzbyB0aGVcbiAgICAvLyBmYWxsYmFjayBzdGF5cyB0eXBlLXByZXNlcnZpbmcgKG51bWJlcnMsIGJvb2xlYW5zKSBsaWtlIHRoZSBzZWdtZW50IHBhdGguXG4gICAgY29uc3QgcmVzb2x2ZSA9IChzKSA9PiB7XG4gICAgICBjb25zdCBtID0gb3B0aW9ucy5maW5kKChvKSA9PiBTdHJpbmcodHlwZW9mIG8gPT09ICdvYmplY3QnID8gby52YWx1ZSA6IG8pID09PSBzKTtcbiAgICAgIHJldHVybiBtID09PSB1bmRlZmluZWQgPyBzIDogdHlwZW9mIG0gPT09ICdvYmplY3QnID8gbS52YWx1ZSA6IG07XG4gICAgfTtcbiAgICByZXR1cm4gPFR3ZWFrU2VsZWN0IGxhYmVsPXtsYWJlbH0gdmFsdWU9e3ZhbHVlfSBvcHRpb25zPXtvcHRpb25zfVxuICAgICAgICAgICAgICAgICAgICAgICAgb25DaGFuZ2U9eyhzKSA9PiBvbkNoYW5nZShyZXNvbHZlKHMpKX0gLz47XG4gIH1cbiAgY29uc3Qgb3B0cyA9IG9wdGlvbnMubWFwKChvKSA9PiAodHlwZW9mIG8gPT09ICdvYmplY3QnID8gbyA6IHsgdmFsdWU6IG8sIGxhYmVsOiBvIH0pKTtcbiAgY29uc3QgaWR4ID0gTWF0aC5tYXgoMCwgb3B0cy5maW5kSW5kZXgoKG8pID0+IG8udmFsdWUgPT09IHZhbHVlKSk7XG4gIGNvbnN0IG4gPSBvcHRzLmxlbmd0aDtcblxuICBjb25zdCBzZWdBdCA9IChjbGllbnRYKSA9PiB7XG4gICAgY29uc3QgciA9IHRyYWNrUmVmLmN1cnJlbnQuZ2V0Qm91bmRpbmdDbGllbnRSZWN0KCk7XG4gICAgY29uc3QgaW5uZXIgPSByLndpZHRoIC0gNDtcbiAgICBjb25zdCBpID0gTWF0aC5mbG9vcigoKGNsaWVudFggLSByLmxlZnQgLSAyKSAvIGlubmVyKSAqIG4pO1xuICAgIHJldHVybiBvcHRzW01hdGgubWF4KDAsIE1hdGgubWluKG4gLSAxLCBpKSldLnZhbHVlO1xuICB9O1xuXG4gIGNvbnN0IG9uUG9pbnRlckRvd24gPSAoZSkgPT4ge1xuICAgIHNldERyYWdnaW5nKHRydWUpO1xuICAgIGNvbnN0IHYwID0gc2VnQXQoZS5jbGllbnRYKTtcbiAgICBpZiAodjAgIT09IHZhbHVlUmVmLmN1cnJlbnQpIG9uQ2hhbmdlKHYwKTtcbiAgICBjb25zdCBtb3ZlID0gKGV2KSA9PiB7XG4gICAgICBpZiAoIXRyYWNrUmVmLmN1cnJlbnQpIHJldHVybjtcbiAgICAgIGNvbnN0IHYgPSBzZWdBdChldi5jbGllbnRYKTtcbiAgICAgIGlmICh2ICE9PSB2YWx1ZVJlZi5jdXJyZW50KSBvbkNoYW5nZSh2KTtcbiAgICB9O1xuICAgIGNvbnN0IHVwID0gKCkgPT4ge1xuICAgICAgc2V0RHJhZ2dpbmcoZmFsc2UpO1xuICAgICAgd2luZG93LnJlbW92ZUV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJtb3ZlJywgbW92ZSk7XG4gICAgICB3aW5kb3cucmVtb3ZlRXZlbnRMaXN0ZW5lcigncG9pbnRlcnVwJywgdXApO1xuICAgIH07XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJtb3ZlJywgbW92ZSk7XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJ1cCcsIHVwKTtcbiAgfTtcblxuICByZXR1cm4gKFxuICAgIDxUd2Vha1JvdyBsYWJlbD17bGFiZWx9PlxuICAgICAgPGRpdiByZWY9e3RyYWNrUmVmfSByb2xlPVwicmFkaW9ncm91cFwiIG9uUG9pbnRlckRvd249e29uUG9pbnRlckRvd259XG4gICAgICAgICAgIGNsYXNzTmFtZT17ZHJhZ2dpbmcgPyAndHdrLXNlZyBkcmFnZ2luZycgOiAndHdrLXNlZyd9PlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1zZWctdGh1bWJcIlxuICAgICAgICAgICAgIHN0eWxlPXt7IGxlZnQ6IGBjYWxjKDJweCArICR7aWR4fSAqICgxMDAlIC0gNHB4KSAvICR7bn0pYCxcbiAgICAgICAgICAgICAgICAgICAgICB3aWR0aDogYGNhbGMoKDEwMCUgLSA0cHgpIC8gJHtufSlgIH19IC8+XG4gICAgICAgIHtvcHRzLm1hcCgobykgPT4gKFxuICAgICAgICAgIDxidXR0b24ga2V5PXtvLnZhbHVlfSB0eXBlPVwiYnV0dG9uXCIgcm9sZT1cInJhZGlvXCIgYXJpYS1jaGVja2VkPXtvLnZhbHVlID09PSB2YWx1ZX0+XG4gICAgICAgICAgICB7by5sYWJlbH1cbiAgICAgICAgICA8L2J1dHRvbj5cbiAgICAgICAgKSl9XG4gICAgICA8L2Rpdj5cbiAgICA8L1R3ZWFrUm93PlxuICApO1xufVxuXG5mdW5jdGlvbiBUd2Vha1NlbGVjdCh7IGxhYmVsLCB2YWx1ZSwgb3B0aW9ucywgb25DaGFuZ2UgfSkge1xuICByZXR1cm4gKFxuICAgIDxUd2Vha1JvdyBsYWJlbD17bGFiZWx9PlxuICAgICAgPHNlbGVjdCBjbGFzc05hbWU9XCJ0d2stZmllbGRcIiB2YWx1ZT17dmFsdWV9IG9uQ2hhbmdlPXsoZSkgPT4gb25DaGFuZ2UoZS50YXJnZXQudmFsdWUpfT5cbiAgICAgICAge29wdGlvbnMubWFwKChvKSA9PiB7XG4gICAgICAgICAgY29uc3QgdiA9IHR5cGVvZiBvID09PSAnb2JqZWN0JyA/IG8udmFsdWUgOiBvO1xuICAgICAgICAgIGNvbnN0IGwgPSB0eXBlb2YgbyA9PT0gJ29iamVjdCcgPyBvLmxhYmVsIDogbztcbiAgICAgICAgICByZXR1cm4gPG9wdGlvbiBrZXk9e3Z9IHZhbHVlPXt2fT57bH08L29wdGlvbj47XG4gICAgICAgIH0pfVxuICAgICAgPC9zZWxlY3Q+XG4gICAgPC9Ud2Vha1Jvdz5cbiAgKTtcbn1cblxuZnVuY3Rpb24gVHdlYWtUZXh0KHsgbGFiZWwsIHZhbHVlLCBwbGFjZWhvbGRlciwgb25DaGFuZ2UgfSkge1xuICByZXR1cm4gKFxuICAgIDxUd2Vha1JvdyBsYWJlbD17bGFiZWx9PlxuICAgICAgPGlucHV0IGNsYXNzTmFtZT1cInR3ay1maWVsZFwiIHR5cGU9XCJ0ZXh0XCIgdmFsdWU9e3ZhbHVlfSBwbGFjZWhvbGRlcj17cGxhY2Vob2xkZXJ9XG4gICAgICAgICAgICAgb25DaGFuZ2U9eyhlKSA9PiBvbkNoYW5nZShlLnRhcmdldC52YWx1ZSl9IC8+XG4gICAgPC9Ud2Vha1Jvdz5cbiAgKTtcbn1cblxuZnVuY3Rpb24gVHdlYWtOdW1iZXIoeyBsYWJlbCwgdmFsdWUsIG1pbiwgbWF4LCBzdGVwID0gMSwgdW5pdCA9ICcnLCBvbkNoYW5nZSB9KSB7XG4gIGNvbnN0IGNsYW1wID0gKG4pID0+IHtcbiAgICBpZiAobWluICE9IG51bGwgJiYgbiA8IG1pbikgcmV0dXJuIG1pbjtcbiAgICBpZiAobWF4ICE9IG51bGwgJiYgbiA+IG1heCkgcmV0dXJuIG1heDtcbiAgICByZXR1cm4gbjtcbiAgfTtcbiAgY29uc3Qgc3RhcnRSZWYgPSBSZWFjdC51c2VSZWYoeyB4OiAwLCB2YWw6IDAgfSk7XG4gIGNvbnN0IG9uU2NydWJTdGFydCA9IChlKSA9PiB7XG4gICAgZS5wcmV2ZW50RGVmYXVsdCgpO1xuICAgIHN0YXJ0UmVmLmN1cnJlbnQgPSB7IHg6IGUuY2xpZW50WCwgdmFsOiB2YWx1ZSB9O1xuICAgIGNvbnN0IGRlY2ltYWxzID0gKFN0cmluZyhzdGVwKS5zcGxpdCgnLicpWzFdIHx8ICcnKS5sZW5ndGg7XG4gICAgY29uc3QgbW92ZSA9IChldikgPT4ge1xuICAgICAgY29uc3QgZHggPSBldi5jbGllbnRYIC0gc3RhcnRSZWYuY3VycmVudC54O1xuICAgICAgY29uc3QgcmF3ID0gc3RhcnRSZWYuY3VycmVudC52YWwgKyBkeCAqIHN0ZXA7XG4gICAgICBjb25zdCBzbmFwcGVkID0gTWF0aC5yb3VuZChyYXcgLyBzdGVwKSAqIHN0ZXA7XG4gICAgICBvbkNoYW5nZShjbGFtcChOdW1iZXIoc25hcHBlZC50b0ZpeGVkKGRlY2ltYWxzKSkpKTtcbiAgICB9O1xuICAgIGNvbnN0IHVwID0gKCkgPT4ge1xuICAgICAgd2luZG93LnJlbW92ZUV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJtb3ZlJywgbW92ZSk7XG4gICAgICB3aW5kb3cucmVtb3ZlRXZlbnRMaXN0ZW5lcigncG9pbnRlcnVwJywgdXApO1xuICAgIH07XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJtb3ZlJywgbW92ZSk7XG4gICAgd2luZG93LmFkZEV2ZW50TGlzdGVuZXIoJ3BvaW50ZXJ1cCcsIHVwKTtcbiAgfTtcbiAgcmV0dXJuIChcbiAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1udW1cIj5cbiAgICAgIDxzcGFuIGNsYXNzTmFtZT1cInR3ay1udW0tbGJsXCIgb25Qb2ludGVyRG93bj17b25TY3J1YlN0YXJ0fT57bGFiZWx9PC9zcGFuPlxuICAgICAgPGlucHV0IHR5cGU9XCJudW1iZXJcIiB2YWx1ZT17dmFsdWV9IG1pbj17bWlufSBtYXg9e21heH0gc3RlcD17c3RlcH1cbiAgICAgICAgICAgICBvbkNoYW5nZT17KGUpID0+IG9uQ2hhbmdlKGNsYW1wKE51bWJlcihlLnRhcmdldC52YWx1ZSkpKX0gLz5cbiAgICAgIHt1bml0ICYmIDxzcGFuIGNsYXNzTmFtZT1cInR3ay1udW0tdW5pdFwiPnt1bml0fTwvc3Bhbj59XG4gICAgPC9kaXY+XG4gICk7XG59XG5cbi8vIFJlbGF0aXZlLWx1bWluYW5jZSBjb250cmFzdCBwaWNrIOKAlCBjaGVja21hcmtzIGRyYXduIG92ZXIgYSBzd2F0Y2ggbmVlZCB0b1xuLy8gcmVhZCBvbiBib3RoICMxMTEgYW5kICNmYWZhZmEgd2l0aG91dCBwZXItb3B0aW9uIGNvbmZpZ3VyYXRpb24uIEhleCBpbnB1dFxuLy8gb25seSAoI3JnYiAvICNycmdnYmIpOyBuYW1lZCBvciByZ2IoKS9oc2woKSBjb2xvcnMgZmFsbCB0aHJvdWdoIHRvIFwibGlnaHRcIi5cbmZ1bmN0aW9uIF9fdHdrSXNMaWdodChoZXgpIHtcbiAgY29uc3QgaCA9IFN0cmluZyhoZXgpLnJlcGxhY2UoJyMnLCAnJyk7XG4gIGNvbnN0IHggPSBoLmxlbmd0aCA9PT0gMyA/IGgucmVwbGFjZSgvLi9nLCAoYykgPT4gYyArIGMpIDogaC5wYWRFbmQoNiwgJzAnKTtcbiAgY29uc3QgbiA9IHBhcnNlSW50KHguc2xpY2UoMCwgNiksIDE2KTtcbiAgaWYgKE51bWJlci5pc05hTihuKSkgcmV0dXJuIHRydWU7XG4gIGNvbnN0IHIgPSAobiA+PiAxNikgJiAyNTUsIGcgPSAobiA+PiA4KSAmIDI1NSwgYiA9IG4gJiAyNTU7XG4gIHJldHVybiByICogMjk5ICsgZyAqIDU4NyArIGIgKiAxMTQgPiAxNDgwMDA7XG59XG5cbmNvbnN0IF9fVHdrQ2hlY2sgPSAoeyBsaWdodCB9KSA9PiAoXG4gIDxzdmcgdmlld0JveD1cIjAgMCAxNCAxNFwiIGFyaWEtaGlkZGVuPVwidHJ1ZVwiPlxuICAgIDxwYXRoIGQ9XCJNMyA3LjIgNS44IDEwIDExIDQuMlwiIGZpbGw9XCJub25lXCIgc3Ryb2tlV2lkdGg9XCIyLjJcIlxuICAgICAgICAgIHN0cm9rZUxpbmVjYXA9XCJyb3VuZFwiIHN0cm9rZUxpbmVqb2luPVwicm91bmRcIlxuICAgICAgICAgIHN0cm9rZT17bGlnaHQgPyAncmdiYSgwLDAsMCwuNzgpJyA6ICcjZmZmJ30gLz5cbiAgPC9zdmc+XG4pO1xuXG4vLyBUd2Vha0NvbG9yIOKAlCBjdXJhdGVkIGNvbG9yL3BhbGV0dGUgcGlja2VyLiBFYWNoIG9wdGlvbiBpcyBlaXRoZXIgYSBzaW5nbGVcbi8vIGhleCBzdHJpbmcgb3IgYW4gYXJyYXkgb2YgMS01IGhleCBzdHJpbmdzOyB0aGUgY2FyZCBhZGFwdHMg4oCUIGEgbG9uZSBjb2xvclxuLy8gcmVuZGVycyBzb2xpZCwgYSBwYWxldHRlIHJlbmRlcnMgY29sb3JzWzBdIGFzIHRoZSBoZXJvIChsZWZ0IH4yLzMpIHdpdGggdGhlXG4vLyByZXN0IHN0YWNrZWQgaW4gYSBzaGFycCBjb2x1bW4gb24gdGhlIHJpZ2h0LiBvbkNoYW5nZSBlbWl0cyB0aGVcbi8vIG9wdGlvbiBpbiB0aGUgc2hhcGUgaXQgd2FzIHBhc3NlZCAoc3RyaW5nIHN0YXlzIHN0cmluZywgYXJyYXkgc3RheXMgYXJyYXkpLlxuLy8gV2l0aG91dCBvcHRpb25zIGl0IGZhbGxzIGJhY2sgdG8gdGhlIG5hdGl2ZSBjb2xvciBpbnB1dCBmb3IgYmFjay1jb21wYXQuXG5mdW5jdGlvbiBUd2Vha0NvbG9yKHsgbGFiZWwsIHZhbHVlLCBvcHRpb25zLCBvbkNoYW5nZSB9KSB7XG4gIGlmICghb3B0aW9ucyB8fCAhb3B0aW9ucy5sZW5ndGgpIHtcbiAgICByZXR1cm4gKFxuICAgICAgPGRpdiBjbGFzc05hbWU9XCJ0d2stcm93IHR3ay1yb3ctaFwiPlxuICAgICAgICA8ZGl2IGNsYXNzTmFtZT1cInR3ay1sYmxcIj48c3Bhbj57bGFiZWx9PC9zcGFuPjwvZGl2PlxuICAgICAgICA8aW5wdXQgdHlwZT1cImNvbG9yXCIgY2xhc3NOYW1lPVwidHdrLXN3YXRjaFwiIHZhbHVlPXt2YWx1ZX1cbiAgICAgICAgICAgICAgIG9uQ2hhbmdlPXsoZSkgPT4gb25DaGFuZ2UoZS50YXJnZXQudmFsdWUpfSAvPlxuICAgICAgPC9kaXY+XG4gICAgKTtcbiAgfVxuICAvLyBOYXRpdmUgPGlucHV0IHR5cGU9Y29sb3I+IGVtaXRzIGxvd2VyY2FzZSBoZXggcGVyIHRoZSBIVE1MIHNwZWMsIHNvXG4gIC8vIGNvbXBhcmUgY2FzZS1pbnNlbnNpdGl2ZWx5LiBTdHJpbmcoKSBndWFyZHMgSlNPTi5zdHJpbmdpZnkodW5kZWZpbmVkKSxcbiAgLy8gd2hpY2ggcmV0dXJucyB0aGUgcHJpbWl0aXZlIHVuZGVmaW5lZCAobm8gLnRvTG93ZXJDYXNlKS5cbiAgY29uc3Qga2V5ID0gKG8pID0+IFN0cmluZyhKU09OLnN0cmluZ2lmeShvKSkudG9Mb3dlckNhc2UoKTtcbiAgY29uc3QgY3VyID0ga2V5KHZhbHVlKTtcbiAgcmV0dXJuIChcbiAgICA8VHdlYWtSb3cgbGFiZWw9e2xhYmVsfT5cbiAgICAgIDxkaXYgY2xhc3NOYW1lPVwidHdrLWNoaXBzXCIgcm9sZT1cInJhZGlvZ3JvdXBcIj5cbiAgICAgICAge29wdGlvbnMubWFwKChvLCBpKSA9PiB7XG4gICAgICAgICAgY29uc3QgY29sb3JzID0gQXJyYXkuaXNBcnJheShvKSA/IG8gOiBbb107XG4gICAgICAgICAgY29uc3QgW2hlcm8sIC4uLnJlc3RdID0gY29sb3JzO1xuICAgICAgICAgIGNvbnN0IHN1cCA9IHJlc3Quc2xpY2UoMCwgNCk7XG4gICAgICAgICAgY29uc3Qgb24gPSBrZXkobykgPT09IGN1cjtcbiAgICAgICAgICByZXR1cm4gKFxuICAgICAgICAgICAgPGJ1dHRvbiBrZXk9e2l9IHR5cGU9XCJidXR0b25cIiBjbGFzc05hbWU9XCJ0d2stY2hpcFwiIHJvbGU9XCJyYWRpb1wiXG4gICAgICAgICAgICAgICAgICAgIGFyaWEtY2hlY2tlZD17b259IGRhdGEtb249e29uID8gJzEnIDogJzAnfVxuICAgICAgICAgICAgICAgICAgICBhcmlhLWxhYmVsPXtjb2xvcnMuam9pbignLCAnKX0gdGl0bGU9e2NvbG9ycy5qb2luKCcgwrcgJyl9XG4gICAgICAgICAgICAgICAgICAgIHN0eWxlPXt7IGJhY2tncm91bmQ6IGhlcm8gfX1cbiAgICAgICAgICAgICAgICAgICAgb25DbGljaz17KCkgPT4gb25DaGFuZ2Uobyl9PlxuICAgICAgICAgICAgICB7c3VwLmxlbmd0aCA+IDAgJiYgKFxuICAgICAgICAgICAgICAgIDxzcGFuPlxuICAgICAgICAgICAgICAgICAge3N1cC5tYXAoKGMsIGopID0+IDxpIGtleT17an0gc3R5bGU9e3sgYmFja2dyb3VuZDogYyB9fSAvPil9XG4gICAgICAgICAgICAgICAgPC9zcGFuPlxuICAgICAgICAgICAgICApfVxuICAgICAgICAgICAgICB7b24gJiYgPF9fVHdrQ2hlY2sgbGlnaHQ9e19fdHdrSXNMaWdodChoZXJvKX0gLz59XG4gICAgICAgICAgICA8L2J1dHRvbj5cbiAgICAgICAgICApO1xuICAgICAgICB9KX1cbiAgICAgIDwvZGl2PlxuICAgIDwvVHdlYWtSb3c+XG4gICk7XG59XG5cbmZ1bmN0aW9uIFR3ZWFrQnV0dG9uKHsgbGFiZWwsIG9uQ2xpY2ssIHNlY29uZGFyeSA9IGZhbHNlIH0pIHtcbiAgcmV0dXJuIChcbiAgICA8YnV0dG9uIHR5cGU9XCJidXR0b25cIiBjbGFzc05hbWU9e3NlY29uZGFyeSA/ICd0d2stYnRuIHNlY29uZGFyeScgOiAndHdrLWJ0bid9XG4gICAgICAgICAgICBvbkNsaWNrPXtvbkNsaWNrfT57bGFiZWx9PC9idXR0b24+XG4gICk7XG59XG5cbk9iamVjdC5hc3NpZ24od2luZG93LCB7XG4gIHVzZVR3ZWFrcywgVHdlYWtzUGFuZWwsIFR3ZWFrU2VjdGlvbiwgVHdlYWtSb3csXG4gIFR3ZWFrU2xpZGVyLCBUd2Vha1RvZ2dsZSwgVHdlYWtSYWRpbywgVHdlYWtTZWxlY3QsXG4gIFR3ZWFrVGV4dCwgVHdlYWtOdW1iZXIsIFR3ZWFrQ29sb3IsIFR3ZWFrQnV0dG9uLFxufSk7XG4iXSwibWFwcGluZ3MiOiJBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBOztBQUVBLE1BQU1BLGNBQWMsR0FBRztBQUN2QjtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0E7QUFDQTtBQUNBO0FBQ0EsQ0FBQzs7QUFFRDtBQUNBO0FBQ0E7QUFDQSxTQUFTQyxTQUFTQSxDQUFDQyxRQUFRLEVBQUU7RUFDM0IsTUFBTSxDQUFDQyxNQUFNLEVBQUVDLFNBQVMsQ0FBQyxHQUFHQyxLQUFLLENBQUNDLFFBQVEsQ0FBQ0osUUFBUSxDQUFDO0VBQ3BEO0VBQ0E7RUFDQTtFQUNBLE1BQU1LLFFBQVEsR0FBR0YsS0FBSyxDQUFDRyxXQUFXLENBQUMsQ0FBQ0MsVUFBVSxFQUFFQyxHQUFHLEtBQUs7SUFDdEQsTUFBTUMsS0FBSyxHQUFHLE9BQU9GLFVBQVUsS0FBSyxRQUFRLElBQUlBLFVBQVUsS0FBSyxJQUFJLEdBQy9EQSxVQUFVLEdBQUc7TUFBRSxDQUFDQSxVQUFVLEdBQUdDO0lBQUksQ0FBQztJQUN0Q04sU0FBUyxDQUFFUSxJQUFJLEtBQU07TUFBRSxH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
})();

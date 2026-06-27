const COLORS = [
    '#e6194b', '#3cb44b', '#4363d8', '#f58231', '#911eb4',
    '#42d4f4', '#f032e6', '#bfef45', '#fabed4', '#469990',
];

const KEY = 'qol-debug';
const VERBOSE_KEY = 'qol-debug-verbose';

let enabled = !(typeof localStorage !== 'undefined' && localStorage.getItem(KEY) === '0');
let verbose = typeof localStorage !== 'undefined' && localStorage.getItem(VERBOSE_KEY) === '1';
let colorIndex = 0;
const nsColors = new Map();

const TRACE_MAX = 500;
const ARG_MAX = 200;
const traceRing = [];
let traceSeq = 0;

export function isDebugEnabled() { return enabled; }

export function setDebugEnabled(on) {
    enabled = !!on;
    if (typeof localStorage === 'undefined') return;
    on ? localStorage.removeItem(KEY) : localStorage.setItem(KEY, '0');
}

export function isVerboseDebugEnabled() { return verbose; }

export function setVerboseDebugEnabled(on) {
    verbose = !!on;
    if (typeof localStorage === 'undefined') return;
    on ? localStorage.setItem(VERBOSE_KEY, '1') : localStorage.removeItem(VERBOSE_KEY);
}

export function createDebug(namespace) {
    if (!nsColors.has(namespace)) nsColors.set(namespace, COLORS[colorIndex++ % COLORS.length]);

    const log = (...args) => {
        captureTrace(namespace, args);
        if (!enabled) return;
        const color = nsColors.get(namespace);
        console.log(`%c${namespace}%c`, `color:${color};font-weight:bold`, 'color:inherit', ...args);
    };

    log.verbose = (...args) => {
        if (!enabled || !verbose) return;
        const color = nsColors.get(namespace);
        console.log(`%c${namespace}%c`, `color:${color};font-weight:bold;opacity:0.7`, 'color:inherit;opacity:0.7', ...args);
    };

    log.extend = (sub) => createDebug(`${namespace}:${sub}`);
    log.namespace = namespace;
    return log;
}

export function elLabel(el) {
    if (!el) return 'null';
    if (el === document.body) return 'BODY';
    const tag = el.tagName?.toLowerCase() || '?';
    const cls = el.className ? '.' + String(el.className).split(/\s+/).slice(0, 2).join('.') : '';
    return tag + cls;
}

export function rectLabel(r) {
    if (!r) return 'none';
    const x = r.x ?? r.left;
    const y = r.y ?? r.top;
    return `(${Math.round(x)},${Math.round(y)} ${Math.round(r.width)}x${Math.round(r.height)})`;
}

export function pointLabel(p) {
    if (!p) return '(?)';
    return `(${Math.round(p.x)},${Math.round(p.y)})`;
}

function traceNow() {
    return typeof performance !== 'undefined' && performance.now ? Math.round(performance.now()) : 0;
}

export function formatTraceArg(arg) {
    if (arg == null) return String(arg);
    const type = typeof arg;
    if (type === 'string') return arg.length > ARG_MAX ? arg.slice(0, ARG_MAX) + '…' : arg;
    if (type === 'number' || type === 'boolean') return String(arg);
    if (typeof Element !== 'undefined' && arg instanceof Element) return elLabel(arg);
    if (type === 'object') {
        if ('width' in arg && ('left' in arg || 'x' in arg)) return rectLabel(arg);
        try { return JSON.stringify(arg).slice(0, ARG_MAX); } catch { return String(arg); }
    }
    return String(arg);
}

function captureTrace(namespace, args) {
    traceRing.push({ seq: traceSeq++, t: traceNow(), ns: namespace, msg: args.map(formatTraceArg).join(' ') });
    if (traceRing.length > TRACE_MAX) traceRing.shift();
}

export function getTrace(filter) {
    if (!filter) return traceRing.slice();
    return traceRing.filter(entry => entry.ns.includes(filter) || entry.msg.includes(filter));
}

export function clearTrace() {
    traceRing.length = 0;
}

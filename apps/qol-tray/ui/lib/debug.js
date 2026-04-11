/**
 * Frontend debug logger with namespace support.
 *
 * Toggle from the Developer > Dev tab or programmatically:
 *   import { setDebugEnabled } from './debug.js';
 *   setDebugEnabled(true);
 */

const COLORS = [
    '#e6194b', '#3cb44b', '#4363d8', '#f58231', '#911eb4',
    '#42d4f4', '#f032e6', '#bfef45', '#fabed4', '#469990',
];

const KEY = 'qol-debug';
let enabled = typeof localStorage !== 'undefined' && localStorage.getItem(KEY) === '1';
let colorIndex = 0;
const nsColors = new Map();

export function isDebugEnabled() { return enabled; }

export function setDebugEnabled(on) {
    enabled = on;
    if (typeof localStorage !== 'undefined') {
        on ? localStorage.setItem(KEY, '1') : localStorage.removeItem(KEY);
    }
}

export function createDebug(namespace) {
    if (!nsColors.has(namespace)) nsColors.set(namespace, COLORS[colorIndex++ % COLORS.length]);

    const log = (...args) => {
        if (!enabled) return;
        const color = nsColors.get(namespace);
        console.log(`%c${namespace}%c`, `color:${color};font-weight:bold`, 'color:inherit', ...args);
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

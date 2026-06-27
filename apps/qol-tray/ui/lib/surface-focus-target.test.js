import { test } from 'node:test';
import assert from 'node:assert/strict';
import {
    SURFACE_CONTROL_COMMIT,
    finishSurfaceFocusTarget,
    focusSurfaceTarget,
    surfaceFocusReturnTarget,
} from './surface-focus-target.js';

function makeEl({ attrs = {}, children = [] } = {}) {
    const el = {
        _attrs: { ...attrs },
        _children: children,
        parentElement: null,
        focused: false,
        events: [],
        getAttribute(name) { return this._attrs[name] ?? null; },
        hasAttribute(name) { return name in this._attrs; },
        matches(selector) { return matches(this, selector); },
        closest(selector) { return matches(this, selector) ? this : (this.parentElement?.closest?.(selector) ?? null); },
        querySelector(selector) { return findDescendant(this, selector); },
        focus() { this.focused = true; },
        dispatchEvent(event) { this.events.push(event.type); return true; },
    };
    for (const child of children) child.parentElement = el;
    return el;
}

function findDescendant(el, selector) {
    for (const child of el._children) {
        if (matches(child, selector)) return child;
        const match = findDescendant(child, selector);
        if (match) return match;
    }
    return null;
}

function matches(el, selector) {
    if (!selector.startsWith('[') || !selector.endsWith(']')) return false;
    const name = selector.slice(1, -1);
    return el.hasAttribute(name);
}

test('focusSurfaceTarget focuses a declared descendant target', () => {
    const target = makeEl({ attrs: { 'data-surface-focus-target': '' } });
    const surface = makeEl({ attrs: { 'data-selected-surface': '' }, children: [target] });
    assert.equal(focusSurfaceTarget(surface), true);
    assert.equal(target.focused, true);
});

test('surfaceFocusReturnTarget prefers explicit return targets', () => {
    const target = makeEl({ attrs: { 'data-surface-focus-target': '', 'data-selected-surface': '' } });
    const slider = makeEl({ attrs: { 'data-selected-surface': '' }, children: [target] });
    const field = makeEl({ attrs: { 'data-surface-focus-return': '', 'data-selected-surface': '' }, children: [slider] });

    assert.equal(surfaceFocusReturnTarget(target), field);
});

test('surfaceFocusReturnTarget falls back to the parent selected surface', () => {
    const target = makeEl({ attrs: { 'data-surface-focus-target': '', 'data-selected-surface': '' } });
    const surface = makeEl({ attrs: { 'data-selected-surface': '' }, children: [target] });

    assert.equal(surfaceFocusReturnTarget(target), surface);
});

test('finishSurfaceFocusTarget commits before returning focus', () => {
    const target = makeEl({ attrs: { 'data-surface-focus-target': '', 'data-selected-surface': '' } });
    const surface = makeEl({ attrs: { 'data-selected-surface': '' }, children: [target] });

    assert.equal(finishSurfaceFocusTarget(target), true);
    assert.deepEqual(target.events, [SURFACE_CONTROL_COMMIT]);
    assert.equal(surface.focused, true);
});

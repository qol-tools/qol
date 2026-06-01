import { test } from 'node:test';
import assert from 'node:assert/strict';
import { handleSurfaceClick } from './surface-click.js';

function makeEl({ tag = 'div', attrs = {}, children = [] } = {}) {
    const el = {
        tagName: tag.toUpperCase(),
        _attrs: { ...attrs },
        _children: children,
        _parent: null,
        getAttribute(name) { return this._attrs[name] ?? null; },
        setAttribute(name, value) { this._attrs[name] = String(value); },
        hasAttribute(name) { return name in this._attrs; },
        closest(selector) { return matchesAny(this, selector) ? this : (this._parent?.closest?.(selector) ?? null); },
    };
    for (const child of children) child._parent = el;
    return el;
}

function matchesAny(el, selector) {
    return selector.split(',').map(s => s.trim()).some(sel => matches(el, sel));
}

function matches(el, sel) {
    if (sel.startsWith('[') && sel.endsWith(']')) {
        const m = sel.slice(1, -1);
        const eq = m.indexOf('=');
        if (eq === -1) return el.hasAttribute(m);
        const name = m.slice(0, eq);
        const value = m.slice(eq + 1).replace(/^"|"$/g, '');
        return el.getAttribute(name) === value;
    }
    return el.tagName === sel.toUpperCase();
}

function makeEvent({ target, currentTarget, shiftKey = false }) {
    return {
        target, currentTarget, shiftKey,
        propagationStopped: false,
        stopPropagation() { this.propagationStopped = true; },
    };
}

test('click on bare inner button reaches onActivate (install-button regression)', () => {
    const button = makeEl({ tag: 'button', attrs: { class: 'install' } });
    const card = makeEl({ tag: 'div', children: [button] });
    let activatedTarget = null;
    handleSurfaceClick(
        { onActivate: (e) => { activatedTarget = e.target; } },
        makeEvent({ target: button, currentTarget: card }),
    );
    assert.equal(activatedTarget, button, 'onActivate must fire for bare inner <button>');
});

test('click on bare inner anchor reaches onActivate', () => {
    const a = makeEl({ tag: 'a' });
    const card = makeEl({ tag: 'div', children: [a] });
    let activated = false;
    handleSurfaceClick(
        { onActivate: () => { activated = true; } },
        makeEvent({ target: a, currentTarget: card }),
    );
    assert.equal(activated, true);
});

test('click on inner input does NOT reach onActivate (form-field guard)', () => {
    const input = makeEl({ tag: 'input' });
    const surface = makeEl({ tag: 'div', children: [input] });
    let activated = false;
    handleSurfaceClick(
        { onActivate: () => { activated = true; } },
        makeEvent({ target: input, currentTarget: surface }),
    );
    assert.equal(activated, false, 'input click must NOT activate parent surface');
});

test('click on inner textarea/select/contenteditable does NOT activate parent', () => {
    for (const tag of ['textarea', 'select']) {
        const child = makeEl({ tag });
        const surface = makeEl({ tag: 'div', children: [child] });
        let activated = false;
        handleSurfaceClick(
            { onActivate: () => { activated = true; } },
            makeEvent({ target: child, currentTarget: surface }),
        );
        assert.equal(activated, false, `${tag} must not activate parent`);
    }
    const ce = makeEl({ tag: 'div', attrs: { contenteditable: 'true' } });
    const surface = makeEl({ tag: 'div', children: [ce] });
    let activated = false;
    handleSurfaceClick(
        { onActivate: () => { activated = true; } },
        makeEvent({ target: ce, currentTarget: surface }),
    );
    assert.equal(activated, false, 'contenteditable must not activate parent');
});

test('successful activation stops propagation (inner Surface-as-button blocks outer Card)', () => {
    const button = makeEl({ tag: 'button' });
    const event = makeEvent({ target: button, currentTarget: button });
    handleSurfaceClick({ onActivate: () => {} }, event);
    assert.equal(event.propagationStopped, true, 'must stopPropagation when onActivate runs');
});

test('no-op surface (no onActivate, no actions, no dive) lets click bubble', () => {
    const inner = makeEl({ tag: 'span' });
    const surface = makeEl({ tag: 'div', children: [inner] });
    const event = makeEvent({ target: inner, currentTarget: surface });
    handleSurfaceClick({}, event);
    assert.equal(event.propagationStopped, false);
});

test('shift+click runs secondary and stops propagation', () => {
    let secondaryRan = false;
    const button = makeEl({ tag: 'button' });
    const event = makeEvent({ target: button, currentTarget: button, shiftKey: true });
    handleSurfaceClick(
        { onSecondaryActivate: () => { secondaryRan = true; } },
        event,
    );
    assert.equal(secondaryRan, true);
    assert.equal(event.propagationStopped, true);
});

test('shift+click without secondary action falls through to primary', () => {
    let primaryRan = false;
    const button = makeEl({ tag: 'button' });
    const event = makeEvent({ target: button, currentTarget: button, shiftKey: true });
    handleSurfaceClick(
        { onActivate: () => { primaryRan = true; } },
        event,
    );
    assert.equal(primaryRan, true, 'shift held but no secondary defined → primary runs (matches pre-fix behavior)');
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { surfaceStatus, effectiveReachable, classifyInteractable } from './surface-audit.js';

test('surfaceStatus mirrors directSurfaces filter precedence', () => {
    const cases = [
        ['all clear is reachable', { visible: true, disabled: false, inert: false, shadowed: false }, 'reachable'],
        ['invisible dominates everything', { visible: false, disabled: true, inert: true, shadowed: true }, 'invisible'],
        ['disabled outranks inert and shadow', { visible: true, disabled: true, inert: true, shadowed: true }, 'disabled'],
        ['inert outranks shadow', { visible: true, disabled: false, inert: true, shadowed: true }, 'inert'],
        ['shadowed when only shadow set', { visible: true, disabled: false, inert: false, shadowed: true }, 'shadowed'],
    ];
    for (const [label, input, expected] of cases) {
        assert.equal(surfaceStatus(input), expected, label);
    }
});

test('effectiveReachable resolves the focus-delegation chain', () => {
    const cases = [
        ['reachable ignores parent', ['reachable', false], true],
        ['shadowed inherits a reachable parent', ['shadowed', true], true],
        ['shadowed blocked by unreachable parent', ['shadowed', false], false],
        ['invisible is never reachable', ['invisible', true], false],
        ['disabled is never reachable', ['disabled', true], false],
        ['inert is never reachable', ['inert', true], false],
    ];
    for (const [label, [status, parent], expected] of cases) {
        assert.equal(effectiveReachable(status, parent), expected, label);
    }
});

test('classifyInteractable maps surface presence and reachability to a verdict', () => {
    const cases = [
        ['no surface ancestor is an orphan', { hasSurface: false, reachable: false }, 'orphan'],
        ['surface present but unreachable', { hasSurface: true, reachable: false }, 'unreachable'],
        ['reachable surface is ok', { hasSurface: true, reachable: true }, 'ok'],
    ];
    for (const [label, input, expected] of cases) {
        assert.equal(classifyInteractable(input), expected, label);
    }
});

test('slider field-group/control/thumb chain stays reachable', () => {
    const fieldGroup = effectiveReachable('reachable', false);
    const control = effectiveReachable('shadowed', fieldGroup);
    const thumb = effectiveReachable('shadowed', control);
    assert.equal(thumb, true, 'thumb reachable through two shadow hops');
    assert.equal(classifyInteractable({ hasSurface: true, reachable: thumb }), 'ok');
});

test('slider thumb goes unreachable when an ancestor surface is hidden', () => {
    const control = effectiveReachable('invisible', true);
    const thumb = effectiveReachable('shadowed', control);
    assert.equal(thumb, false, 'a hidden ancestor breaks the chain');
    assert.equal(classifyInteractable({ hasSurface: true, reachable: thumb }), 'unreachable');
});

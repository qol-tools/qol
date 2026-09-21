import test from 'node:test';
import assert from 'node:assert/strict';
import { isLiveField, isLiveNumberField } from '../../lib/qol-config.js';
import { isSliderNumberField } from './field-rules.js';

test('a field is live when it declares active_query', () => {
    assert.equal(isLiveField({ id: 'volume', kind: 'number', active_query: 'volume' }), true);
    assert.equal(isLiveField({ id: 'output', kind: 'select', active_query: 'output_status' }), true);
    assert.equal(isLiveField({ id: 'volume', kind: 'number' }), false);
    assert.equal(isLiveField({ id: 'volume', kind: 'number', active_query: '' }), false);
});

test('only number fields are live numbers', () => {
    assert.equal(isLiveNumberField({ id: 'volume', kind: 'number', active_query: 'volume' }), true);
    assert.equal(
        isLiveNumberField({ id: 'output', kind: 'select', active_query: 'output_status' }),
        false,
    );
    assert.equal(isLiveNumberField({ id: 'volume', kind: 'number' }), false);
});

test('a live number renders as a slider', () => {
    assert.equal(isSliderNumberField({ kind: 'number', active_query: 'volume' }), true);
    assert.equal(isSliderNumberField({ kind: 'number', variant: 'wide_slider' }), true);
    assert.equal(
        isSliderNumberField({ kind: 'number', variant: 'default', number: { min: 1, max: 100 } }),
        false,
    );
});

import { test } from 'node:test';
import assert from 'node:assert/strict';
import { selectOptions } from './select-options.js';

test('static selects pass options and labels through unchanged', () => {
    const field = {
        options: ['notification', 'toast'],
        option_labels: { notification: 'System Notification' },
    };
    const { options, labels } = selectOptions(field, 'toast', null);
    assert.deepEqual(options, ['notification', 'toast']);
    assert.deepEqual(labels, { notification: 'System Notification' });
});

test('query selects merge labeled seeds, rows, and the current value', () => {
    const field = {
        options: [],
        option_labels: { default: 'System Default' },
        query: 'audio_sources',
    };
    const rows = [
        { value: 'alsa_input.foo', label: 'Built-in Mic' },
        { value: 'default', label: 'ignored, contract label wins' },
        { label: 'no value, skipped' },
    ];
    const { options, labels } = selectOptions(field, 'gone_device', rows);
    assert.deepEqual(options, ['gone_device', 'default', 'alsa_input.foo']);
    assert.deepEqual(labels, {
        default: 'System Default',
        'alsa_input.foo': 'Built-in Mic',
    });
});

test('query selects tolerate missing rows while polls are in flight', () => {
    const field = { options: [], option_labels: { default: 'System Default' }, query: 'q' };
    const { options } = selectOptions(field, 'default', undefined);
    assert.deepEqual(options, ['default']);
});

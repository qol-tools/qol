import { escapeHtml } from '../utils/escape-html.js';

export function renderShortcutLegend(shortcuts) {
    const items = shortcuts.map(({ key, label }) =>
        `<span class="help-item"><kbd>${escapeHtml(key)}</kbd> ${escapeHtml(label)}</span>`
    ).join('');
    return `<div class="help">${items}</div>`;
}

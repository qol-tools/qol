export function renderShortcutLegend(shortcuts) {
    const items = shortcuts.map(({ key, label }) =>
        `<span class="help-item"><kbd>${key}</kbd> ${label}</span>`
    ).join('');
    return `<div class="help">${items}</div>`;
}

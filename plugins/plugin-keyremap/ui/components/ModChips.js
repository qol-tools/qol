import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';

const DEFAULT_MODS = ['ctrl', 'shift', 'alt', 'cmd'];

export function ModChips({ selected = [], onChange, mods = DEFAULT_MODS }) {
    const clear = useCallback(() => {
        onChange([]);
    }, [onChange]);

    const toggle = useCallback((mod) => {
        const next = selected.includes(mod)
            ? selected.filter(m => m !== mod)
            : [...selected, mod];
        onChange(next);
    }, [selected, onChange]);

    return html`
        <div class="mod-toggles">
            <button
                class="mod-chip ${selected.length === 0 ? 'active' : ''}"
                onClick=${(e) => { e.preventDefault(); clear(); }}
            >(none)</button>
            ${mods.map(mod => html`
                <button
                    class="mod-chip ${selected.includes(mod) ? 'active' : ''}"
                    onClick=${(e) => { e.preventDefault(); toggle(mod); }}
                >${mod}</button>
            `)}
        </div>
    `;
}

export function ModChipsStatic({ mods = [] }) {
    if (mods.length === 0) {
        return html`<span class="mod-chip-static">(none)</span>`;
    }
    return html`${mods.map(m => html`<span class="mod-chip-static">${m}</span>`)}`;
}

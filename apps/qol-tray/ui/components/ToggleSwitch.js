import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';

export function ToggleSwitch({ checked, onChange, label }) {
    const toggle = useCallback(() => onChange(!checked), [checked, onChange]);
    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        toggle();
    }, [toggle]);

    return html`
        <div class="toggle-inline" onClick=${toggle}>
            <div class="toggle-track ${checked ? 'on' : ''}" tabIndex="0" role="switch"
                aria-checked=${checked} onKeyDown=${onKeyDown}>
                <div class="toggle-thumb" />
            </div>
            <span class="toggle-inline-label">${label}</span>
        </div>
    `;
}

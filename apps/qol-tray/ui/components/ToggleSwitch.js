import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';
import { Surface } from './Surface.js';

export function ToggleSwitch({ checked, onChange, label, ...rest }) {
    const toggle = useCallback(() => onChange(!checked), [checked, onChange]);

    return html`
        <${Surface} className="toggle-inline" onActivate=${toggle} role="switch"
            aria-checked=${checked} ...${rest}>
            <div class="toggle-track ${checked ? 'on' : ''}">
                <div class="toggle-thumb" />
            </div>
            <span class="toggle-inline-label">${label}</span>
        <//>
    `;
}

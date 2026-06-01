import { html } from '../lib/html.js';

export function EnableToggle({ enabled, onChange }) {
    return html`
        <section class="card">
            <label class="mode-option">
                <input type="checkbox" checked=${enabled} onChange=${(e) => onChange(e.target.checked)} />
                <span class="mode-copy">
                    <strong>Enable Remapping</strong>
                    <small>Master switch. When off, all events pass through unmodified.</small>
                </span>
            </label>
        </section>
    `;
}

import { html } from '../lib/html.js';

export function SaveBar({ onSave, saveStatus, saveError, warnings }) {
    return html`
        <footer class="actions">
            ${warnings.length > 0 && html`
                <div id="save-warnings">
                    ${warnings.map(w => html`<span class="warning-line">\u26a0 ${w}</span>`)}
                    <span class="warning-hint">Click save again to confirm.</span>
                </div>
            `}
            <div class="actions-row">
                <button class="save" onClick=${onSave}>Save Settings</button>
                <span class=${saveError ? 'error' : ''}>${saveStatus}</span>
            </div>
        </footer>
    `;
}

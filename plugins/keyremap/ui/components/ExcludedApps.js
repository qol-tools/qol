import { html } from '../lib/html.js';
import { useCallback } from 'preact/hooks';
import { AppPicker } from './AppPicker.js';
import { appNameFor } from '../hooks/useApps.js';

export function ExcludedApps({ apps, allApps, onAdd, onRemove }) {
    const handleAdd = useCallback((bundleId) => {
        if (!bundleId || apps.includes(bundleId)) return;
        onAdd(bundleId);
    }, [apps, onAdd]);

    return html`
        <section class="card">
            <h2>Excluded Apps</h2>
            <p>Apps that handle Ctrl natively (terminals, IDEs). Remapping is bypassed for these bundle IDs.</p>
            <div class="item-list">
                ${apps.length === 0 && html`<div class="empty-state">No excluded apps. All apps will have remapping active.</div>`}
                ${apps.map((bid, i) => html`
                    <div class="item-row">
                        <img class="app-icon" src="/api/icon/${encodeURIComponent(bid)}" width="20" height="20" onError=${(e) => e.target.style.display = 'none'} />
                        ${appNameFor(bid, allApps)
                            ? html`
                                <span class="app-name">${appNameFor(bid, allApps)}</span>
                                <span class="app-separator">\u2014</span>
                                <span class="app-bid">${bid}</span>
                            `
                            : html`<span>${bid}</span>`
                        }
                        <button class="btn-remove" onClick=${() => onRemove(i)}>\u00d7</button>
                    </div>
                `)}
            </div>
            <${AppPicker} excludedApps=${apps} onSelect=${handleAdd} />
        </section>
    `;
}

import { html } from '../../lib/html.js';
import { Card, CardGrid } from '../../lib/components/Card.js';

export function StoreCardGrid({ className, onDeselect, children, ...rest }) {
    const cls = ['plugin-grid-store card-grid--zoom', className].filter(Boolean).join(' ');
    return html`<${CardGrid} className=${cls} onDeselect=${onDeselect} ...${rest}>${children}<//>`;
}

export function StoreCard({ name, version, description, installed, installing, hasUpdate, index, selected, onSelect, onActivate, ...rest }) {
    const cls = ['plugin-card', installed && 'installed', installing && 'installing'].filter(Boolean).join(' ');
    const versionDisplay = hasUpdate ? `v${version.from} -> v${version.to}` : `v${version.current || version}`;
    return html`
        <${Card} className=${cls} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate} ...${rest}>
            <h3 data-selected-text="">${name}</h3>
            <div class="version ${hasUpdate ? 'has-update' : ''}" data-selected-text="">${versionDisplay}</div>
            <div class="description" data-selected-text="">${description}</div>
            <div class="button-group">
                ${installing
                    ? html`<button class="refresh-btn spinning" disabled></button>`
                    : installed
                        ? html`<span class="installed-badge">${hasUpdate ? 'Update Available' : 'Installed'}</span>`
                        : html`<button class="btn btn-primary install" style="width:100%">Install</button>`
                }
            </div>
        <//>
    `;
}

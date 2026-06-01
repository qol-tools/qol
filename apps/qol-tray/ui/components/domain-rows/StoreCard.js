import { html } from '../../lib/html.js';
import { Card, CardGrid } from '../../lib/components/Card.js';

export function StoreCardGrid({ className, onDeselect, children, ...rest }) {
    const cls = ['plugin-grid-store card-grid--zoom', className].filter(Boolean).join(' ');
    return html`<${CardGrid} className=${cls} onDeselect=${onDeselect} ...${rest}>${children}<//>`;
}

export function StoreCard({ name, version, description, installed, installing, hasUpdate, devLinked, index, selected, onSelect, onActivate, ...rest }) {
    const cls = ['plugin-card', installed && 'installed', installing && 'installing', hasUpdate && 'has-update'].filter(Boolean).join(' ');
    const versionDisplay = storeVersionLabel(version, hasUpdate);
    return html`
        <${Card} className=${cls} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate} ...${rest}>
            <h3 data-selected-text="">${name}</h3>
            <div class="version ${hasUpdate ? 'has-update' : ''}" data-selected-text="">${versionDisplay}</div>
            <div class="description" data-selected-text="">${description}</div>
            <div class="button-group">
                ${storeCardAction({ installing, devLinked, hasUpdate, installed })}
            </div>
        <//>
    `;
}

function storeVersionLabel(version, hasUpdate) {
    if (hasUpdate) return `v${version.from} -> v${version.to}`;
    const current = version?.current || version;
    return current ? `v${current}` : '';
}

function storeCardAction({ installing, devLinked, hasUpdate, installed }) {
    if (installing) return html`<button class="refresh-btn spinning" disabled></button>`;
    if (devLinked) return html`<span class="installed-badge dev-linked-badge">Dev linked</span>`;
    if (hasUpdate) return html`<button class="btn btn-primary update" style="width:100%">Update</button>`;
    if (installed) return html`<span class="installed-badge">Installed</span>`;
    return html`<button class="btn btn-primary install" style="width:100%">Install</button>`;
}

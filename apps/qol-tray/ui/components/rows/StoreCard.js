import { html } from '../../lib/html.js';
import { Surface } from '../Surface.js';

export function StoreCardGrid({ className, onDeselect, children, ...rest }) {
    const cls = ['plugin-grid-store grid-cards grid-cards--zoom', className].filter(Boolean).join(' ');
    const onFocusOut = onDeselect ? (e) => {
        if (!e.relatedTarget || !e.currentTarget.contains(e.relatedTarget)) onDeselect();
    } : undefined;
    return html`<div class=${cls} onFocusOut=${onFocusOut} ...${rest}>${children}</div>`;
}

export function StoreCard({ name, version, description, installed, installing, hasUpdate, index, selected, onSelect, onActivate, ...rest }) {
    const cls = ['plugin-card', installed && 'installed', installing && 'installing'].filter(Boolean).join(' ');
    const versionDisplay = hasUpdate ? `v${version.from} -> v${version.to}` : `v${version.current || version}`;
    return html`
        <${Surface} className=${cls} index=${index} selected=${selected} onSelect=${onSelect} onActivate=${onActivate} ...${rest}>
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

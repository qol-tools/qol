import { html } from '../../lib/html.js';
import { Surface } from '../Surface.js';
import { SurfaceContainer } from '../SurfaceContainer.js';

const STATUS_ACCENT = { linked: 'success', local: 'warning', installed: 'accent' };

export function DevPluginRow({ name, path, status, pluginId, badges, meta, action, overlay, index, selected, onSelect, onActivate, ...rest }) {
    const statusCls = status ? `status-${status}` : '';
    const cls = ['plugin-row table-list-row', statusCls].filter(Boolean).join(' ');
    return html`
        <${Surface} className=${cls} index=${index} selected=${selected} onSelect=${onSelect}
            onActivate=${onActivate} data-accent=${STATUS_ACCENT[status]}
            data-status=${status} data-plugin-id=${pluginId} ...${rest}>
            <div class="plugin-main table-grid">
                <div class="plugin-info table-col">
                    <div class="plugin-copy">
                        <div class="plugin-title-row">
                            <span class="plugin-name" data-selected-text="">${name}</span>
                        </div>
                        ${path && html`<span class="plugin-path" data-selected-text="">${path}</span>`}
                        ${meta}
                    </div>
                    ${badges}
                </div>
                ${action && html`
                    <${SurfaceContainer} className="plugin-action-column table-col">
                        ${action}
                    <//>
                `}
            </div>
            ${overlay && html`<div class="plugin-build-overlay-host">${overlay}</div>`}
        <//>
    `;
}

import { html } from '../../lib/html.js';
import { useState, useCallback, useEffect, useRef } from 'preact/hooks';
import { TableRow } from '../TableRow.js';
import { SurfaceContainer } from '../SurfaceContainer.js';
import { Button } from '../Button.js';
import { useClickOutside } from '../../hooks/useClickOutside.js';

const STATUS_ACCENT = { linked: 'success', local: 'warning', installed: 'accent' };

export function DevPluginRow({ name, path, status, pluginId, badges, meta, actions, actionIcon, overlay, index, selected, onSelect, ...rest }) {
    const [menuOpen, setMenuOpen] = useState(false);
    const containerRef = useRef(null);
    const menuRef = useRef(null);
    const close = useCallback(() => {
        setMenuOpen(false);
        const row = containerRef.current?.querySelector('[data-selected-surface]');
        if (row) requestAnimationFrame(() => row.focus({ preventScroll: true }));
    }, []);
    useClickOutside(containerRef, menuOpen, close);

    useEffect(() => {
        if (!menuOpen) return;
        const first = menuRef.current?.querySelector('[data-selected-surface]');
        if (first) first.focus({ preventScroll: true });
    }, [menuOpen]);

    const activate = useCallback(() => {
        if (!actions?.length) return;
        setMenuOpen(o => !o);
    }, [actions]);

    const statusCls = status ? `status-${status}` : '';
    const cls = ['plugin-row', statusCls].filter(Boolean).join(' ');
    return html`
        <div ref=${containerRef} style="position:relative">
            <${TableRow} className=${cls} index=${index} selected=${selected} onSelect=${onSelect}
                onActivate=${activate} accent=${STATUS_ACCENT[status]}
                data-status=${status} data-plugin-id=${pluginId} ...${rest}>
                <div class="plugin-info">
                    <div class="plugin-copy">
                        <div class="plugin-title-row">
                            <span class="plugin-name" data-selected-text="">${name}</span>
                        </div>
                        ${path && html`<span class="plugin-path" data-selected-text="">${path}</span>`}
                        ${meta}
                    </div>
                    ${badges}
                </div>
                <div class="plugin-action-column">
                    <div class="plugin-action-zone">
                        ${actionIcon || html`<img class="list-row-action-icon" src="assets/qol-tray.png?v=1" alt="" />`}
                    </div>
                </div>
                ${overlay && html`<div class="plugin-build-overlay-host">${overlay}</div>`}
            <//>
            ${menuOpen && actions?.length > 0 && html`
                <${SurfaceContainer} ref=${menuRef} className="dev-plugin-row-actions">
                    ${actions.map(a => html`
                        <${Button} key=${a.label} variant="btn-ghost"
                            onActivate=${() => { close(); a.run?.(); }}>
                            ${a.label}
                        <//>
                    `)}
                <//>
            `}
        </div>
    `;
}

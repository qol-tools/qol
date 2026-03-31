import { html } from '../lib/html.js';
import { useEffect, useMemo, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../views/plugin-config/context.js';
import { prettyLabel } from '../auto-config/heuristics.js';
import { materializeIn } from '../lib/dissolve.js';

const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    profile: 'Profile',
    logs: 'Logs',
    dev: 'Developer'
};
const DIVIDER_BEFORE = new Set(['hotkeys', 'profile', 'dev']);

let prevMode = null;

export function SidebarNav({
    activeViewId,
    viewOrder,
    pluginOpen,
    onViewClick,
    onBack,
    profileSyncHealth = 'not_configured',
}) {
    const ctx = usePluginConfigContext();
    const itemsRef = useRef(null);

    const isPluginConfig = pluginOpen && ctx?.sections?.length > 0;
    const isPluginUi = pluginOpen && ctx?.mode === 'ui';
    const mode = isPluginConfig ? 'config' : isPluginUi ? 'ui' : 'views';

    const items = useMemo(() => {
        if (mode === 'config') {
            return ctx.sections.map((s, i) => ({
                type: 'item',
                key: s.id,
                label: s.label || prettyLabel(s.id),
                active: i === ctx.activeSectionIndex,
                onClick: () => ctx.setActiveSectionIndex(i),
            }));
        }
        if (mode === 'ui') return [];
        return viewOrder.flatMap(id => {
            const next = [];
            if (DIVIDER_BEFORE.has(id)) {
                next.push({ type: 'divider', key: `divider:${id}` });
            }
            next.push({
                type: 'item',
                key: id,
                label: VIEW_LABELS[id] || id,
                active: id === activeViewId,
                onClick: () => onViewClick(id),
                trailing: id === 'profile'
                    ? html`<span
                        class="sidebar-status-dot"
                        data-health=${profileSyncHealth}
                        title=${profileStatusTitle(profileSyncHealth)}
                    ></span>`
                    : null,
            });
            return next;
        });
    }, [mode, ctx, activeViewId, viewOrder, onViewClick, profileSyncHealth]);
    useEffect(() => {
        const el = itemsRef.current;
        if (prevMode !== null && prevMode !== mode && el?.offsetHeight > 0) {
            materializeIn(el);
        }
        prevMode = mode;
    });

    const header = pluginOpen
        ? html`<div class="sidebar-header"><button class="sidebar-back" tabIndex="-1" onClick=${onBack}>\u2190 Back</button></div>`
        : html`<div class="sidebar-header"><span class="sidebar-logo">QoL Tray</span></div>`;

    return html`
        ${header}
        <div class="sidebar-nav">
            <div class="sidebar-items" ref=${itemsRef}>
                ${items.map(item => html`
                    ${item.type === 'divider'
                        ? html`<div key=${item.key} class="sidebar-divider" aria-hidden="true"></div>`
                        : html`
                            <div
                                key=${item.key}
                                class="sidebar-item ${item.active ? 'active' : ''}"
                                onClick=${item.onClick}>
                                <div class="sidebar-item-inner">
                                    <span>${item.label}</span>
                                    ${item.trailing}
                                </div>
                            </div>
                        `}
                `)}
            </div>
        </div>
    `;
}

function profileStatusTitle(health) {
    if (health === 'healthy') return 'Cloud sync healthy';
    if (health === 'attention') return 'Cloud sync needs review';
    if (health === 'error') return 'Cloud sync error';
    return 'Cloud sync not configured';
}

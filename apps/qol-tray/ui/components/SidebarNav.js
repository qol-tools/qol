import { html } from '../lib/html.js';
import { useEffect, useMemo, useRef } from 'preact/hooks';
import { usePluginConfigContext } from '../views/plugin-config/context.js';
import { prettyLabel } from '../auto-config/heuristics.js';
import { dissolveIn, materializeIn, DISSOLVE_PRESETS } from '../lib/dissolve.js';

const VIEW_LABELS = {
    plugins: 'Plugins',
    store: 'Store',
    hotkeys: 'Hotkeys',
    shortcuts: 'Shortcuts',
    'task-runner': 'Task Runner',
    logs: 'Logs',
    dev: 'Developer'
};

let prevMode = null;

export function SidebarNav({ activeViewId, viewOrder, pluginOpen, onViewClick, onBack }) {
    const ctx = usePluginConfigContext();
    const itemsRef = useRef(null);

    const isPluginConfig = pluginOpen && ctx?.sections?.length > 0;
    const isPluginUi = pluginOpen && ctx?.mode === 'ui';
    const mode = isPluginConfig ? 'config' : isPluginUi ? 'ui' : 'views';

    const items = useMemo(() => {
        if (mode === 'config') {
            return ctx.sections.map((s, i) => ({
                key: s.id,
                label: s.label || prettyLabel(s.id),
                active: i === ctx.activeSectionIndex,
                onClick: () => ctx.setActiveSectionIndex(i),
            }));
        }
        if (mode === 'ui') return [];
        return viewOrder.map(id => ({
            key: id,
            label: VIEW_LABELS[id] || id,
            active: id === activeViewId,
            onClick: () => onViewClick(id),
        }));
    }, [mode, ctx, activeViewId, viewOrder, onViewClick]);
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
                    <div
                        key=${item.key}
                        class="sidebar-item ${item.active ? 'active' : ''}"
                        data-selected-surface=""
                        data-selected=${item.active ? 'true' : 'false'}
                        data-selected-surface-edge-highlight="none"
                        data-selected-surface-priority="-1"
                        onClick=${item.onClick}>
                        <span data-selected-text="">${item.label}</span>
                    </div>
                `)}
            </div>
        </div>
    `;
}

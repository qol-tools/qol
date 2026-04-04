import { html } from '../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import { useSidebarContext } from './app/sidebar-context.js';
import { materializeIn } from '../lib/dissolve.js';

function itemSetKey(items) {
    return items.map(i => i.key || i.id || '').join('\0');
}

export function SidebarNav() {
    const { items, header } = useSidebarContext();
    const itemsRef = useRef(null);
    const prevSetKeyRef = useRef(null);
    const setKey = itemSetKey(items);

    useEffect(() => {
        const el = itemsRef.current;
        if (prevSetKeyRef.current !== null && prevSetKeyRef.current !== setKey && el?.offsetHeight > 0) {
            materializeIn(el);
        }
        prevSetKeyRef.current = setKey;
    });

    return html`
        ${header || html`<div class="sidebar-header"><span class="sidebar-logo">QoL Tray</span></div>`}
        <div class="sidebar-nav">
            <div class="sidebar-items" ref=${itemsRef}>
                ${items.map(item => html`
                    ${item.type === 'divider'
                        ? html`<div key=${item.key || item.id} class="sidebar-divider" aria-hidden="true"></div>`
                        : html`
                            <div key=${item.key || item.id}
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

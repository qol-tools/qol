import { html } from '../../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import * as devModule from './index.js';
import { renderShortcutLegend } from '../../components/shortcut-legend.js';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'toggle' },
    { key: 'm', label: 'menu' },
    { key: 'Esc', label: 'close menu' },
    { key: 'r', label: 'discover' },
    { key: '⌘R', label: 'reload' }
];

export function DevView() {
    const containerRef = useRef(null);

    useEffect(() => {
        const el = containerRef.current;
        if (!el) return;

        const footer = document.getElementById('content-footer');
        if (footer) footer.innerHTML = renderShortcutLegend(SHORTCUTS);

        devModule.render(el);

        return () => {
            if (devModule.destroy) devModule.destroy();
            if (footer) footer.innerHTML = '';
        };
    }, []);

    DevView.handleKey = devModule.handleKey;
    DevView.isBlocking = devModule.isBlocking || (() => false);

    return html`<div ref=${containerRef} style="flex:1;min-height:0;display:flex;flex-direction:column"></div>`;
}

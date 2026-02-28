import { html } from '../lib/html.js';
import { useEffect, useRef } from 'preact/hooks';
import * as devModule from './dev.js';
import { renderShortcutLegend } from '../components/shortcut-legend.js';

const SHORTCUTS = [
    { key: '↑↓', label: 'navigate' },
    { key: 'Enter', label: 'toggle' },
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
        if (devModule.onFocus) devModule.onFocus();

        // Restore window focus/blur lifecycle (dev.js manages its own SSE subscription)
        const onFocus = () => { if (devModule.onFocus) devModule.onFocus(); };
        const onBlur = () => { if (devModule.onBlur) devModule.onBlur(); };
        window.addEventListener('focus', onFocus);
        window.addEventListener('blur', onBlur);

        return () => {
            if (devModule.onBlur) devModule.onBlur();
            if (footer) footer.innerHTML = '';
            window.removeEventListener('focus', onFocus);
            window.removeEventListener('blur', onBlur);
        };
    }, []);

    DevView.handleKey = devModule.handleKey;
    DevView.isBlocking = devModule.isBlocking || (() => false);

    return html`<div ref=${containerRef}></div>`;
}

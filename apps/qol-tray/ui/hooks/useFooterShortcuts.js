import { useEffect } from 'preact/hooks';
import { renderShortcutLegend } from '../components/shortcut-legend.js';

export function useFooterShortcuts(shortcuts) {
    useEffect(() => {
        const el = document.getElementById('content-footer');
        if (el) el.innerHTML = renderShortcutLegend(shortcuts);
        return () => { if (el) el.innerHTML = ''; };
    }, []);
}

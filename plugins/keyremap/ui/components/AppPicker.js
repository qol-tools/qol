import { html } from '../lib/html.js';
import { useState, useRef, useEffect, useCallback } from 'preact/hooks';
import { useApps } from '../hooks/useApps.js';

export function AppPicker({ excludedApps, onSelect }) {
    const [query, setQuery] = useState('');
    const [open, setOpen] = useState(false);
    const ref = useRef(null);
    const { apps, loading, fetchApps } = useApps();

    useEffect(() => {
        const handler = (e) => {
            if (ref.current && !ref.current.contains(e.target)) setOpen(false);
        };
        document.addEventListener('click', handler);
        return () => document.removeEventListener('click', handler);
    }, []);

    const onFocus = useCallback(() => {
        setOpen(true);
        fetchApps();
    }, [fetchApps]);

    const onInput = useCallback((e) => {
        setQuery(e.target.value);
        if (!open) setOpen(true);
    }, [open]);

    const onKeyDown = useCallback((e) => {
        if (e.key === 'Enter') {
            e.preventDefault();
            const v = query.trim();
            if (!v) return;
            onSelect(v);
            setQuery('');
        }
        if (e.key === 'Escape') {
            setOpen(false);
            e.target.blur();
        }
    }, [query, onSelect]);

    const selectApp = useCallback((bid) => {
        setQuery('');
        onSelect(bid);
    }, [onSelect]);

    const q = query.trim().toLowerCase();
    const filtered = apps
        ? apps.filter(a => !q || a.name.toLowerCase().includes(q) || a.bundle_id.toLowerCase().includes(q))
        : [];

    return html`
        <div class="add-row app-picker-container" ref=${ref}>
            <input
                type="text"
                class="text-input"
                placeholder="Search apps or type bundle ID..."
                autocomplete="off"
                value=${query}
                onFocus=${onFocus}
                onInput=${onInput}
                onKeyDown=${onKeyDown}
            />
            <button class="btn-add" onClick=${() => { if (query.trim()) { onSelect(query.trim()); setQuery(''); } }}>+ Add</button>
            ${open && html`
                <div class="app-picker-dropdown">
                    ${loading && html`<div class="app-picker-loading">Loading apps...</div>`}
                    ${!loading && filtered.length === 0 && html`<div class="app-picker-empty">${q ? 'No matches' : 'No apps found'}</div>`}
                    ${!loading && filtered.map(app => {
                        const disabled = excludedApps.includes(app.bundle_id);
                        return html`
                            <div class="app-picker-item ${disabled ? 'disabled' : ''}"
                                onClick=${() => !disabled && selectApp(app.bundle_id)}>
                                <img src="/api/icon/${encodeURIComponent(app.bundle_id)}" width="24" height="24" onError=${(e) => e.target.style.display = 'none'} />
                                <div class="app-picker-info">
                                    <span class="app-picker-name">${app.name}</span>
                                    <span class="app-picker-bid">${app.bundle_id}</span>
                                </div>
                            </div>
                        `;
                    })}
                </div>
            `}
        </div>
    `;
}

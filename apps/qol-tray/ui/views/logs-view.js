import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { PageHeader } from '../components/PageHeader.js';

const TABS = [
    { id: 'live', label: 'Live Log' },
    { id: 'suppressed', label: 'Suppressed' },
];

const POLL_INTERVAL = 5000;

export function LogsView({ active }) {
    const [activeTab, setActiveTab] = useState('live');
    const [entries, setEntries] = useState([]);
    const [suppressed, setSuppressed] = useState({});
    const [selectedIndex, setSelectedIndex] = useState(0);
    const contentRef = useRef(null);

    const fetchEntries = useCallback(async () => {
        try {
            const res = await fetch('/api/logs/entries');
            if (res.ok) setEntries(await res.json());
        } catch (_) {}
    }, []);

    const fetchSuppressed = useCallback(async () => {
        try {
            const res = await fetch('/api/logs/suppressed');
            if (res.ok) setSuppressed(await res.json());
        } catch (_) {}
    }, []);

    useEffect(() => {
        if (!active) return;
        fetchEntries();
        fetchSuppressed();
        const id = setInterval(fetchEntries, POLL_INTERVAL);
        return () => clearInterval(id);
    }, [active, fetchEntries, fetchSuppressed]);

    useEffect(() => {
        if (active && contentRef.current) {
            contentRef.current.focus();
        }
    }, [active]);

    useEffect(() => {
        setSelectedIndex(0);
    }, [activeTab]);

    const unsuppress = useCallback(async (key) => {
        try {
            await fetch(`/api/logs/unsuppress/${encodeURIComponent(key)}`, { method: 'POST' });
            fetchSuppressed();
        } catch (_) {}
    }, [fetchSuppressed]);

    const switchTab = useCallback((direction) => {
        const idx = TABS.findIndex(t => t.id === activeTab);
        const next = (idx + direction + TABS.length) % TABS.length;
        setActiveTab(TABS[next].id);
    }, [activeTab]);

    const itemCount = activeTab === 'live'
        ? entries.length
        : Object.keys(suppressed).length;

    const onKeyDown = useCallback((e) => {
        switch (e.key) {
            case 'ArrowLeft':
                e.preventDefault();
                switchTab(-1);
                break;
            case 'ArrowRight':
                e.preventDefault();
                switchTab(1);
                break;
            case 'ArrowUp':
            case 'k':
                e.preventDefault();
                setSelectedIndex(i => Math.max(0, i - 1));
                break;
            case 'ArrowDown':
            case 'j':
                e.preventDefault();
                setSelectedIndex(i => Math.min(itemCount - 1, i + 1));
                break;
            case 'Enter':
                if (activeTab === 'suppressed') {
                    const keys = Object.keys(suppressed);
                    if (keys[selectedIndex]) unsuppress(keys[selectedIndex]);
                }
                break;
        }
    }, [switchTab, itemCount, activeTab, suppressed, selectedIndex, unsuppress]);

    useEffect(() => {
        const el = contentRef.current;
        if (!el) return;
        const selected = el.querySelector('[data-selected="true"]');
        if (selected) selected.scrollIntoView({ block: 'nearest' });
    }, [selectedIndex]);

    return html`
        <${PageHeader} title="Logs" subtitle="Production error log" />
        <div class="logs-tabs" role="tablist">
            ${TABS.map(tab => html`
                <button
                    key=${tab.id}
                    class="logs-tab ${activeTab === tab.id ? 'active' : ''}"
                    role="tab"
                    aria-selected=${activeTab === tab.id}
                    tabIndex=${activeTab === tab.id ? 0 : -1}
                    onClick=${() => setActiveTab(tab.id)}
                    onKeyDown=${(e) => {
                        if (e.key === 'ArrowLeft') { e.preventDefault(); switchTab(-1); }
                        if (e.key === 'ArrowRight') { e.preventDefault(); switchTab(1); }
                    }}
                >${tab.label}</button>
            `)}
        </div>
        <div
            class="logs-content"
            ref=${contentRef}
            tabIndex="0"
            onKeyDown=${onKeyDown}
            role="tabpanel"
        >
            ${activeTab === 'live' && html`<${LiveLog} entries=${entries} selectedIndex=${selectedIndex} />`}
            ${activeTab === 'suppressed' && html`<${SuppressedList} items=${suppressed} onUnsuppress=${unsuppress} selectedIndex=${selectedIndex} />`}
        </div>
    `;
}

function LiveLog({ entries, selectedIndex }) {
    const reversed = [...entries].reverse();
    if (reversed.length === 0) {
        return html`<div class="logs-empty">No log entries for today</div>`;
    }
    return html`
        <div class="logs-entries" role="list">
            ${reversed.map((entry, i) => html`<${LogEntryRow} key=${i} entry=${entry} selected=${i === selectedIndex} />`)}
        </div>
    `;
}

function LogEntryRow({ entry, selected }) {
    const time = entry.ts ? entry.ts.split('T')[1] || entry.ts : '';
    const levelClass = entry.level === 'startup' ? 'level-startup' : entry.suppressed ? 'level-suppressed' : 'level-error';
    const loc = entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0' ? entry.loc : '';
    return html`
        <div class="log-entry ${levelClass} ${selected ? 'selected' : ''}" role="listitem" data-selected=${selected}>
            <span class="log-time">${time}</span>
            <span class="log-level">${entry.level?.toUpperCase()}</span>
            <span class="log-src">${entry.src}</span>
            <span class="log-msg">${entry.msg}</span>
            ${entry.count > 1 && html`<span class="log-count">${'\u00d7'}${entry.count}</span>`}
            ${entry.suppressed && html`<span class="log-badge-suppressed">suppressed</span>`}
            ${loc && html`<span class="log-loc">${loc}</span>`}
        </div>
    `;
}

function SuppressedList({ items, onUnsuppress, selectedIndex }) {
    const keys = Object.keys(items);
    if (keys.length === 0) {
        return html`<div class="logs-empty">No suppressed errors</div>`;
    }
    return html`
        <div class="logs-suppressed-list" role="list">
            ${keys.map((key, i) => html`
                <${SuppressedRow}
                    key=${key}
                    sigKey=${key}
                    entry=${items[key]}
                    onUnsuppress=${onUnsuppress}
                    selected=${i === selectedIndex}
                />
            `)}
        </div>
    `;
}

function SuppressedRow({ sigKey, entry, onUnsuppress, selected }) {
    return html`
        <div class="suppressed-entry ${selected ? 'selected' : ''}" role="listitem" data-selected=${selected}>
            <div class="suppressed-header">
                <span class="suppressed-src">${entry.source || entry.src || '?'}</span>
                <span class="suppressed-count">${'\u00d7'}${entry.count}</span>
                <button
                    class="suppressed-unsuppress"
                    tabIndex=${selected ? 0 : -1}
                    onClick=${() => onUnsuppress(sigKey)}
                >Unsuppress</button>
            </div>
            <div class="suppressed-msg">${entry.last_message || ''}</div>
            <div class="suppressed-meta">
                First: ${entry.first_seen || '?'} · Last: ${entry.last_seen || '?'} · ${entry.version || ''}
            </div>
        </div>
    `;
}

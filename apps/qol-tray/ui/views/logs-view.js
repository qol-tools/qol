import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { matchesQuery } from '../utils/collections.js';
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
    const { searchQuery } = usePaletteContext();

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

    const filteredEntries = useMemo(
        () => searchQuery
            ? entries.filter(e => matchesQuery([e.msg, e.src, e.key, e.level], searchQuery))
            : entries,
        [entries, searchQuery]
    );

    const suppressedKeys = useMemo(() => Object.keys(suppressed), [suppressed]);
    const filteredSuppressedKeys = useMemo(
        () => searchQuery
            ? suppressedKeys.filter(k => {
                const e = suppressed[k];
                return matchesQuery([k, e?.last_message, e?.source, e?.src], searchQuery);
            })
            : suppressedKeys,
        [suppressedKeys, suppressed, searchQuery]
    );

    const itemCount = activeTab === 'live'
        ? filteredEntries.length
        : filteredSuppressedKeys.length;

    useEffect(() => {
        setSelectedIndex(0);
    }, [activeTab, searchQuery]);

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

    const handleKey = useCallback((event) => {
        switch (event.key) {
            case 'ArrowLeft':
                event.preventDefault();
                switchTab(-1);
                break;
            case 'ArrowRight':
                event.preventDefault();
                switchTab(1);
                break;
            case 'ArrowUp':
            case 'k':
                event.preventDefault();
                setSelectedIndex(i => Math.max(0, i - 1));
                break;
            case 'ArrowDown':
            case 'j':
                event.preventDefault();
                setSelectedIndex(i => Math.min(itemCount - 1, i + 1));
                break;
            case 'Enter':
                if (activeTab === 'suppressed') {
                    event.preventDefault();
                    const key = filteredSuppressedKeys[selectedIndex];
                    if (key) unsuppress(key);
                }
                break;
        }
    }, [switchTab, itemCount, activeTab, filteredSuppressedKeys, selectedIndex, unsuppress]);

    useRegisterViewKeyboard('logs', handleKey);

    const commands = useMemo(() => [
        { id: 'refresh', label: 'Refresh Logs', action: () => { fetchEntries(); fetchSuppressed(); } },
        { id: 'live-tab', label: 'Show Live Log', action: () => setActiveTab('live') },
        { id: 'suppressed-tab', label: 'Show Suppressed', action: () => setActiveTab('suppressed') },
    ], [fetchEntries, fetchSuppressed]);
    useRegisterCommands('logs', commands);

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
                    tabIndex="-1"
                    aria-selected=${activeTab === tab.id}
                    onClick=${() => setActiveTab(tab.id)}
                >${tab.label}</button>
            `)}
        </div>
        <div class="logs-content" ref=${contentRef} role="tabpanel">
            ${activeTab === 'live' && html`<${LiveLog} entries=${filteredEntries} selectedIndex=${selectedIndex} />`}
            ${activeTab === 'suppressed' && html`<${SuppressedList}
                keys=${filteredSuppressedKeys}
                items=${suppressed}
                onUnsuppress=${unsuppress}
                selectedIndex=${selectedIndex}
            />`}
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

function SuppressedList({ keys, items, onUnsuppress, selectedIndex }) {
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
                    tabIndex="-1"
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

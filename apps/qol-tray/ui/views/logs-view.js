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

const LEVEL_CONFIG = {
    startup: { label: 'STARTUP', cls: 'level-startup' },
    error: { label: 'ERROR', cls: 'level-error' },
};

function levelInfo(entry) {
    if (entry.level === 'startup') return LEVEL_CONFIG.startup;
    if (entry.suppressed) return { label: 'SUPPRESSED', cls: 'level-suppressed' };
    return LEVEL_CONFIG[entry.level] || { label: (entry.level || '').toUpperCase(), cls: 'level-error' };
}

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

    const tabCounts = {
        live: filteredEntries.length,
        suppressed: filteredSuppressedKeys.length,
    };

    return html`
        <${PageHeader} title="Logs" subtitle="Error log and suppression management" />
        <div class="logs-tabs" role="tablist">
            ${TABS.map(tab => html`
                <button
                    key=${tab.id}
                    class="logs-tab ${activeTab === tab.id ? 'active' : ''}"
                    role="tab"
                    tabIndex="-1"
                    aria-selected=${activeTab === tab.id}
                    onClick=${() => setActiveTab(tab.id)}
                >
                    ${tab.label}
                    ${tabCounts[tab.id] > 0 && html`<span class="logs-tab-count">${tabCounts[tab.id]}</span>`}
                </button>
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
        return html`<${EmptyState} message="No log entries for today" hint="Errors will appear here when they occur" />`;
    }
    return html`
        <div class="logs-entries" role="list">
            ${reversed.map((entry, i) => html`<${LogEntryRow} key=${i} entry=${entry} selected=${i === selectedIndex} />`)}
        </div>
    `;
}

function LogEntryRow({ entry, selected }) {
    const time = entry.ts ? entry.ts.split('T')[1] || entry.ts : '';
    const { label, cls } = levelInfo(entry);
    const loc = entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0' ? entry.loc : '';
    return html`
        <div class="log-entry ${selected ? 'selected' : ''}" role="listitem" data-selected=${selected}>
            <span class="log-time">${time}</span>
            <span class="log-level-badge ${cls}">${label}</span>
            <span class="log-src">${entry.src || ''}</span>
            <span class="log-msg">${entry.msg}</span>
            <span class="log-count">${entry.count > 1 ? `\u00d7${entry.count}` : ''}</span>
            <span class="log-loc">${loc}</span>
        </div>
    `;
}

function SuppressedList({ keys, items, onUnsuppress, selectedIndex }) {
    if (keys.length === 0) {
        return html`<${EmptyState} message="No suppressed errors" hint="Errors that repeat ${'\u2265'}5 times are auto-suppressed" />`;
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
                <span class="suppressed-key">${sigKey}</span>
                <span class="suppressed-count-badge">${'\u00d7'}${entry.count}</span>
                <button
                    class="btn btn-sm suppressed-unsuppress"
                    tabIndex="-1"
                    onClick=${() => onUnsuppress(sigKey)}
                >Unsuppress</button>
            </div>
            ${entry.last_message && html`<div class="suppressed-msg">${entry.last_message}</div>`}
            <div class="suppressed-meta">
                <span class="suppressed-meta-item">
                    <span class="suppressed-meta-label">First</span>
                    <span>${formatTimestamp(entry.first_seen)}</span>
                </span>
                <span class="suppressed-meta-sep">${'\u00b7'}</span>
                <span class="suppressed-meta-item">
                    <span class="suppressed-meta-label">Last</span>
                    <span>${formatTimestamp(entry.last_seen)}</span>
                </span>
                ${entry.version && html`
                    <span class="suppressed-meta-sep">${'\u00b7'}</span>
                    <span class="suppressed-version">${entry.version}</span>
                `}
            </div>
        </div>
    `;
}

function EmptyState({ message, hint }) {
    return html`
        <div class="logs-empty-state">
            <div class="logs-empty-icon">
                <svg viewBox="0 0 24 24" width="32" height="32" fill="none" stroke="currentColor" stroke-width="1.5">
                    <path d="M9 12h6M12 9v6M21 12a9 9 0 1 1-18 0 9 9 0 0 1 18 0z" />
                </svg>
            </div>
            <div class="logs-empty-message">${message}</div>
            ${hint && html`<div class="logs-empty-hint">${hint}</div>`}
        </div>
    `;
}

function formatTimestamp(ts) {
    if (!ts) return '?';
    return ts.replace('T', ' ');
}

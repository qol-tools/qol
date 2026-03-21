import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef } from 'preact/hooks';
import { PageHeader } from '../components/PageHeader.js';

const TABS = [
    { id: 'live', label: 'Live Log' },
    { id: 'suppressed', label: 'Suppressed' },
];

export function LogsView() {
    const [activeTab, setActiveTab] = useState('live');
    const [entries, setEntries] = useState([]);
    const [suppressed, setSuppressed] = useState({});
    const intervalRef = useRef(null);

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
        fetchEntries();
        fetchSuppressed();
        intervalRef.current = setInterval(fetchEntries, 3000);
        return () => clearInterval(intervalRef.current);
    }, [fetchEntries, fetchSuppressed]);

    const unsuppress = useCallback(async (key) => {
        try {
            await fetch(`/api/logs/unsuppress/${encodeURIComponent(key)}`, { method: 'POST' });
            fetchSuppressed();
        } catch (_) {}
    }, [fetchSuppressed]);

    return html`
        <${PageHeader} title="Logs" subtitle="Production error log" />
        <div class="logs-tabs">
            ${TABS.map(tab => html`
                <button
                    key=${tab.id}
                    class="logs-tab ${activeTab === tab.id ? 'active' : ''}"
                    onClick=${() => setActiveTab(tab.id)}
                >${tab.label}</button>
            `)}
        </div>
        <div class="logs-content">
            ${activeTab === 'live' && html`<${LiveLog} entries=${entries} />`}
            ${activeTab === 'suppressed' && html`<${SuppressedList} items=${suppressed} onUnsuppress=${unsuppress} />`}
        </div>
    `;
}

function LiveLog({ entries }) {
    const reversed = [...entries].reverse();
    if (reversed.length === 0) {
        return html`<div class="logs-empty">No log entries for today</div>`;
    }
    return html`
        <div class="logs-entries">
            ${reversed.map((entry, i) => html`<${LogEntryRow} key=${i} entry=${entry} />`)}
        </div>
    `;
}

function LogEntryRow({ entry }) {
    const time = entry.ts ? entry.ts.split('T')[1] || entry.ts : '';
    const levelClass = entry.level === 'startup' ? 'level-startup' : entry.suppressed ? 'level-suppressed' : 'level-error';
    return html`
        <div class="log-entry ${levelClass}">
            <span class="log-time">${time}</span>
            <span class="log-level">${entry.level?.toUpperCase()}</span>
            <span class="log-src">${entry.src}</span>
            <span class="log-msg">${entry.msg}</span>
            ${entry.count > 1 && html`<span class="log-count">${'\u00d7'}${entry.count}</span>`}
            ${entry.suppressed && html`<span class="log-badge-suppressed">suppressed</span>`}
            ${entry.loc && html`<span class="log-loc">${entry.loc}</span>`}
        </div>
    `;
}

function SuppressedList({ items, onUnsuppress }) {
    const keys = Object.keys(items);
    if (keys.length === 0) {
        return html`<div class="logs-empty">No suppressed errors</div>`;
    }
    return html`
        <div class="logs-suppressed-list">
            ${keys.map(key => html`
                <${SuppressedRow} key=${key} sigKey=${key} entry=${items[key]} onUnsuppress=${onUnsuppress} />
            `)}
        </div>
    `;
}

function SuppressedRow({ sigKey, entry, onUnsuppress }) {
    return html`
        <div class="suppressed-entry">
            <div class="suppressed-header">
                <span class="suppressed-src">${entry.source || entry.src || '?'}</span>
                <span class="suppressed-count">${'\u00d7'}${entry.count}</span>
                <button class="suppressed-unsuppress" onClick=${() => onUnsuppress(sigKey)}>Unsuppress</button>
            </div>
            <div class="suppressed-msg">${entry.last_message || ''}</div>
            <div class="suppressed-meta">
                First: ${entry.first_seen || '?'} · Last: ${entry.last_seen || '?'} · ${entry.version || ''}
            </div>
        </div>
    `;
}

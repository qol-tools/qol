import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../components/app/view-keyboard-context.js';
import { useListKeyboard } from '../hooks/useListKeyboard.js';
import { matchesQuery } from '../utils/collections.js';
import { ViewTabs } from '../components/ViewTabs.js';
import { Modal, ModalFooter } from '../components/ModalPreact.js';
import { CodeBlock } from '../components/CodeBlock.js';
import { toast } from '../lib/toast.js';

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
    const [entries, setEntries] = useState([]);
    const [suppressed, setSuppressed] = useState({});
    const [selectedIndex, setSelectedIndex] = useState(0);
    const [expandedKeys, setExpandedKeys] = useState(new Set());
    const [detailEntry, setDetailEntry] = useState(null);
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

    const collapsedEntries = useMemo(() => collapseEntries(filteredEntries), [filteredEntries]);

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

    const onTabActivate = useCallback((tabId) => {
        const count = tabId === 'live' ? filteredEntries.length : filteredSuppressedKeys.length;
        if (count > 0) {
            setSelectedIndex(0);
            requestAnimationFrame(() => focusContentSurface(0));
        }
    }, [filteredEntries.length, filteredSuppressedKeys.length]);

    const vtRef = useRef(null);

    const unsuppress = useCallback(async (key) => {
        try {
            await fetch(`/api/logs/unsuppress/${encodeURIComponent(key)}`, { method: 'POST' });
            fetchSuppressed();
        } catch (_) {}
    }, [fetchSuppressed]);

    const toggleExpand = useCallback((key) => {
        setExpandedKeys(prev => {
            const next = new Set(prev);
            if (next.has(key)) next.delete(key);
            else next.add(key);
            return next;
        });
    }, []);

    const openLogDir = useCallback(async () => {
        try { await fetch('/api/logs/open-dir', { method: 'POST' }); } catch (_) {}
    }, []);

    const onEdit = useCallback(() => {
        const vt = vtRef.current;
        if (!vt) return;
        if (vt.activeTab === 'live') {
            const entry = collapsedEntries[selectedIndex];
            if (entry) {
                document.activeElement?.blur();
                setDetailEntry(entry);
                vt.setZone('detail');
            }
        }
        if (vt.activeTab === 'suppressed') {
            const key = filteredSuppressedKeys[selectedIndex];
            if (key) toggleExpand(key);
        }
    }, [collapsedEntries, filteredSuppressedKeys, selectedIndex, toggleExpand]);

    const itemCount = useMemo(() => {
        const vt = vtRef.current;
        const tab = vt?.activeTab || 'live';
        return tab === 'live' ? collapsedEntries.length : filteredSuppressedKeys.length;
    });

    const listHandler = useListKeyboard({
        surfaceSelector: '.logs-content [data-selected-surface]',
        itemCount,
        selectedIndex,
        setSelectedIndex,
        onEdit,
    });

    const closeDetail = useCallback(() => {
        setDetailEntry(null);
        vtRef.current?.setZone('content');
    }, []);

    const handleKey = useCallback((event) => {
        const vt = vtRef.current;
        if (vt?.zone === 'detail') return;
        if (vt?.handleKey(event)) return;
        listHandler(event);
    }, [listHandler]);

    const isBlocking = useCallback(() => false, []);
    useRegisterViewKeyboard('logs', handleKey, isBlocking);

    const commands = useMemo(() => [
        { id: 'refresh', label: 'Refresh Logs', action: () => { fetchEntries(); fetchSuppressed(); } },
        { id: 'open-dir', label: 'Open Log Directory', action: openLogDir },
        { id: 'live-tab', label: 'Show Live Log', action: () => vtRef.current?.switchTab('live') },
        { id: 'suppressed-tab', label: 'Show Suppressed', action: () => vtRef.current?.switchTab('suppressed') },
    ], [fetchEntries, fetchSuppressed, openLogDir]);
    useRegisterCommands('logs', commands);

    const trailingTab = html`
        <button class="btn btn-sm btn-ghost logs-action-btn" data-selected-surface=""
            data-selected="false" onClick=${openLogDir}>Open log folder</button>
    `;

    const tabsWithCounts = useMemo(() => TABS.map(tab => ({
        ...tab,
        count: tab.id === 'live' ? collapsedEntries.length : filteredSuppressedKeys.length,
    })), [collapsedEntries.length, filteredSuppressedKeys.length]);

    return html`
        <${ViewTabs} title="Logs" subtitle="Error log and suppression management"
            tabs=${tabsWithCounts} onActivate=${onTabActivate} trailing=${trailingTab} vtRef=${vtRef}>
            ${(vt) => html`
                <div class="logs-content" ref=${contentRef}>
                    ${vt.activeTab === 'live' && html`<${LiveLog} entries=${collapsedEntries} selectedIndex=${vt.zone === 'content' ? selectedIndex : -1}
                        onEntryClick=${(entry) => { document.activeElement?.blur(); setDetailEntry(entry); vt.setZone('detail'); }} />`}
                    ${vt.activeTab === 'suppressed' && html`<${SuppressedList}
                        keys=${filteredSuppressedKeys}
                        items=${suppressed}
                        onUnsuppress=${unsuppress}
                        selectedIndex=${vt.zone === 'content' ? selectedIndex : -1}
                        expandedKeys=${expandedKeys}
                        onToggleExpand=${toggleExpand}
                    />`}
                </div>
            `}
        <//>
        ${detailEntry && html`<${LogDetailModal} entry=${detailEntry} onClose=${closeDetail} />`}
    `;
}

function focusContentSurface(index) {
    const el = document.querySelector(`.logs-content [data-selected-surface][data-index="${index}"]`);
    if (el) el.focus();
}

function collapseEntries(entries) {
    const seen = new Map();
    const result = [];
    for (let i = entries.length - 1; i >= 0; i--) {
        const entry = entries[i];
        const key = entry.key || `${entry.src}:${entry.msg}`;
        const existing = seen.get(key);
        if (existing) {
            existing.count = Math.max(existing.count, entry.count || 1);
            continue;
        }
        const collapsed = { ...entry, count: entry.count || 1 };
        seen.set(key, collapsed);
        result.push(collapsed);
    }
    return result;
}

function LiveLog({ entries, selectedIndex, onEntryClick }) {
    if (entries.length === 0) {
        return html`<${EmptyState} message="No log entries for today" hint="Errors will appear here when they occur" />`;
    }
    return html`
        <div class="logs-list" role="list">
            ${entries.map((entry, i) => html`<${LogEntryRow} key=${entry.key || i} entry=${entry} index=${i} selected=${i === selectedIndex} onClick=${() => onEntryClick(entry)} />`)}
        </div>
    `;
}

function countSeverity(count) {
    if (count >= 25) return 'critical';
    if (count >= 10) return 'high';
    if (count >= 3) return 'moderate';
    return '';
}

function LogEntryRow({ entry, index, selected, onClick }) {
    const time = entry.ts ? entry.ts.split('T')[1] || entry.ts : '';
    const { label, cls } = levelInfo(entry);
    const loc = entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0' ? entry.loc : '';
    const severity = entry.level === 'error' ? countSeverity(entry.count) : '';
    return html`
        <div class="log-row" role="listitem"
             data-selected-surface="" data-selected=${selected ? 'true' : 'false'}
             data-index=${String(index)}
             data-level=${cls} data-severity=${severity || undefined}
             onClick=${onClick}>
            <div class="log-row-top">
                <span class="log-time">${time}</span>
                <span class="log-level-badge ${cls}">${label}</span>
                <span class="log-src" data-selected-text="">${entry.src || ''}</span>
                ${loc && html`<span class="log-loc">${loc}</span>`}
                ${entry.count > 1 && html`<span class="log-count">${'\u00d7'}${entry.count}</span>`}
            </div>
            <div class="log-row-bottom">
                <span class="log-msg" data-selected-text="">${entry.msg}</span>
            </div>
        </div>
    `;
}

function SuppressedList({ keys, items, onUnsuppress, selectedIndex, expandedKeys, onToggleExpand }) {
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
                    index=${i}
                    onUnsuppress=${onUnsuppress}
                    selected=${i === selectedIndex}
                    expanded=${expandedKeys.has(key)}
                    onToggle=${() => onToggleExpand(key)}
                />
            `)}
        </div>
    `;
}

function SuppressedRow({ sigKey, entry, index, onUnsuppress, selected, expanded, onToggle }) {
    return html`
        <div class="suppressed-entry ${selected ? 'selected' : ''} ${expanded ? 'expanded' : ''}"
             role="listitem" data-selected-surface="" data-selected=${selected ? 'true' : 'false'} data-index=${String(index)} onClick=${onToggle}>
            <div class="suppressed-header">
                <span class="suppressed-expand-icon">${expanded ? '\u25be' : '\u25b8'}</span>
                <span class="suppressed-key">${sigKey}</span>
                <span class="suppressed-count-badge">${'\u00d7'}${entry.count}</span>
                <button
                    class="btn btn-sm suppressed-unsuppress"
                    tabIndex="-1"
                    onClick=${(e) => { e.stopPropagation(); onUnsuppress(sigKey); }}
                >Unsuppress</button>
            </div>
            ${expanded && html`
                ${entry.last_message && html`<div class="suppressed-msg">${entry.last_message}</div>`}
                <div class="suppressed-detail">
                    ${entry.source && html`
                        <div class="suppressed-detail-row">
                            <span class="suppressed-meta-label">Source</span>
                            <span class="suppressed-detail-value">${entry.source}</span>
                        </div>
                    `}
                    ${entry.location && html`
                        <div class="suppressed-detail-row">
                            <span class="suppressed-meta-label">Location</span>
                            <span class="suppressed-detail-value mono">${entry.location}</span>
                        </div>
                    `}
                </div>
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
            `}
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

function formatLogDetail(entry) {
    const lines = [];
    if (entry.ts) lines.push(`Time:     ${entry.ts.replace('T', ' ')}`);
    if (entry.level) lines.push(`Level:    ${entry.level.toUpperCase()}`);
    if (entry.src) lines.push(`Source:   ${entry.src}`);
    if (entry.key) lines.push(`Key:      ${entry.key}`);
    if (entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0') lines.push(`Location: ${entry.loc}`);
    if (entry.count > 1) lines.push(`Count:    ${entry.count}`);
    if (entry.v) lines.push(`Version:  ${entry.v}`);
    if (entry.commit) lines.push(`Commit:   ${entry.commit}`);
    lines.push('');
    lines.push(entry.msg || '');
    return lines.join('\n');
}

function LogDetailModal({ entry, onClose }) {
    const copy = useCallback(() => {
        navigator.clipboard.writeText(formatLogDetail(entry));
        toast('success', 'Copied to clipboard');
    }, [entry]);

    return html`
        <${Modal} open=${true} onClose=${onClose} size="xl" dismissOnBackdrop=${true} className="edit-modal">
            <div class="edit-modal-content" tabIndex="-1">
                <h3>Log Entry</h3>
                <${CodeBlock} text=${formatLogDetail(entry)} />
                <${ModalFooter} actions=${[
                    { label: 'Close', kbd: 'Esc', onClick: onClose },
                    { label: 'Copy', kbd: 'C', variant: 'btn-primary', onClick: copy },
                ]} />
            </div>
        <//>
    `;
}

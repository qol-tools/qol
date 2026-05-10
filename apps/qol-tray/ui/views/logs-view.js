import { html } from '../lib/html.js';
import { useState, useEffect, useCallback, useRef, useMemo } from 'preact/hooks';
import { usePaletteContext } from '../palette/context.js';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { useRegisterViewKeyboard } from '../app/view-keyboard-context.js';
import { useListKeyboard } from '../lib/hooks/useListKeyboard.js';
import { matchesQuery } from '../utils/collections.js';
import { ViewTabs } from '../components/ViewTabs.js';
import { Button } from '../lib/components/Button.js';
import { EmptyState } from '../lib/components/EmptyState.js';
import { ListGroup } from '../lib/components/ListRow.js';
import { LogRow, LogDetailContent } from '../components/domain-rows/LogRow.js';
import { PageHeader } from '../components/PageHeader.js';
import { SurfaceContainer } from '../lib/components/SurfaceContainer.js';
import { SuppressedRow } from '../components/domain-rows/SuppressedRow.js';

import { createSharedSlot } from '../lib/shared-slot.js';
export const detailSlot = createSharedSlot({ entry: null });

const TABS = [
    { id: 'live', label: 'Live Log' },
    { id: 'suppressed', label: 'Suppressed' },
];

const POLL_INTERVAL = 5000;

function extractTime(entry) {
    return entry.ts ? entry.ts.split('T')[1] || entry.ts : '';
}

function extractLoc(entry) {
    return entry.loc && entry.loc !== 'unknown:0' && entry.loc !== ':0' ? entry.loc : '';
}

function levelName(entry) {
    if (entry.level === 'startup') return 'startup';
    if (entry.suppressed) return 'suppressed';
    return entry.level || 'error';
}

function countSeverity(count) {
    if (count >= 25) return 'critical';
    if (count >= 10) return 'high';
    if (count >= 3) return 'moderate';
    return '';
}

export function LogsView({ active }) {
    const [entries, setEntries] = useState([]);
    const [suppressed, setSuppressed] = useState({});
    const [selectedIndex, setSelectedIndex] = useState(-1);
    const [expandedKeys, setExpandedKeys] = useState(new Set());
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

    const onTabActivate = useCallback(() => {
        setSelectedIndex(0);
    }, []);

    const onContentBlur = useCallback(() => {
        setSelectedIndex(-1);
    }, []);

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
            if (entry) detailSlot.set({ entry });
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
        itemCount,
        selectedIndex,
        onEdit,
    });

    const handleKey = useCallback((event) => {
        if (document.activeElement?.closest('[role="tablist"]')) return;
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

    const trailingTab = html`<${Button} variant="btn-ghost" small className="logs-action-btn" onActivate=${openLogDir}>Open log folder<//>`;

    const tabsWithCounts = useMemo(() => TABS.map(tab => ({
        ...tab,
        count: tab.id === 'live' ? collapsedEntries.length : filteredSuppressedKeys.length,
    })), [collapsedEntries.length, filteredSuppressedKeys.length]);

    return html`
        <${ViewTabs} title="Logs" subtitle="Error log and suppression management"
            tabs=${tabsWithCounts} onActivate=${onTabActivate} onContentBlur=${onContentBlur} trailing=${trailingTab} vtRef=${vtRef}>
            ${(vt) => html`
                <div class="logs-content" ref=${contentRef}>
                    ${vt.activeTab === 'live' && html`<${LiveLog} entries=${collapsedEntries} selectedIndex=${selectedIndex}
                        setSelectedIndex=${setSelectedIndex}
                        onEntryClick=${(entry) => detailSlot.set({ entry })} />`}
                    ${vt.activeTab === 'suppressed' && html`<${SuppressedList}
                        keys=${filteredSuppressedKeys}
                        items=${suppressed}
                        onUnsuppress=${unsuppress}
                        selectedIndex=${selectedIndex}
                        setSelectedIndex=${setSelectedIndex}
                        expandedKeys=${expandedKeys}
                        onToggleExpand=${toggleExpand}
                    />`}
                </div>
            `}
        <//>
    `;
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

function LiveLog({ entries, selectedIndex, setSelectedIndex, onEntryClick }) {
    if (entries.length === 0) {
        return html`<${EmptyState} message="No log entries for today" hint="Errors will appear here when they occur" />`;
    }
    return html`
        <${ListGroup} className="logs-list" role="list">
            ${entries.map((entry, i) => {
                const level = levelName(entry);
                return html`<${LogRow} key=${entry.key || i}
                    time=${extractTime(entry)} level=${level} src=${entry.src} msg=${entry.msg}
                    loc=${extractLoc(entry)} count=${entry.count} severity=${entry.level === 'error' ? countSeverity(entry.count) : ''}
                    index=${i} selected=${i === selectedIndex} onSelect=${setSelectedIndex}
                    onActivate=${() => onEntryClick(entry)} />`;
            })}
        <//>
    `;
}

function SuppressedList({ keys, items, onUnsuppress, selectedIndex, setSelectedIndex, expandedKeys, onToggleExpand }) {
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
                    expanded=${expandedKeys.has(key)}
                    index=${i}
                    selected=${i === selectedIndex}
                    onSelect=${setSelectedIndex}
                    onToggle=${() => onToggleExpand(key)}
                    onUnsuppress=${onUnsuppress}
                />
            `)}
        </div>
    `;
}

export function LogDetailSubPage({ slot }) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);

    const { entry } = slot.get();
    if (!entry) {
        return html`<div class="view-container content-shell">
            <${PageHeader} title="Log Detail" subtitle="Select a log entry to view" />
        </div>`;
    }
    return html`
        <div class="view-container content-shell">
            <${PageHeader} title="Log Detail" subtitle=${`${entry.level} — ${entry.src}`} />
            <div class="view-body content-shell-body">
                <div class="content-shell-inner">
                    <${SurfaceContainer} className="content-frame">
                        <${LogDetailContent} entry=${entry} />
                    <//>
                </div>
            </div>
        </div>
    `;
}

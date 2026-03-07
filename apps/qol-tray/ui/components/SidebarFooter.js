import { html } from '../lib/html.js';
import { useState, useRef, useEffect } from 'preact/hooks';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress, toProgressScale } from '../utils/progress.js';

export function SidebarFooter({ version, updateState, isDevMode, onAction, worktrees, defaultWorktree, setDefaultWorktree }) {
    if (!version) return null;
    const status = updateState?.status || 'idle';

    if (isDevMode) return renderDev(version, updateState, status, onAction, worktrees, defaultWorktree, setDefaultWorktree);
    return renderStable(version, updateState, status, onAction);
}

function visiblePercent(value) {
    return clampPercent(value);
}

function WorktreePicker({ style, options, defaultWorktree, onSelect, onClose, toggleButtonRef }) {
    const [query, setQuery] = useState('');
    const [highlightIdx, setHighlightIdx] = useState(0);
    const inputRef = useRef(null);
    const containerRef = useRef(null);
    const filtered = query
        ? options.filter(o => o.branch.toLowerCase().includes(query.toLowerCase()))
        : options;
    useEffect(() => {
        inputRef.current?.focus();
        function onMouseDown(e) {
            if (toggleButtonRef?.current?.contains(e.target)) return;
            if (containerRef.current && !containerRef.current.contains(e.target)) onClose();
        }
        document.addEventListener('mousedown', onMouseDown);
        return () => document.removeEventListener('mousedown', onMouseDown);
    }, []);
    function handleKey(e) {
        if (e.key === 'ArrowDown') { e.preventDefault(); setHighlightIdx(i => Math.min(i + 1, filtered.length - 1)); return; }
        if (e.key === 'ArrowUp') { e.preventDefault(); setHighlightIdx(i => Math.max(i - 1, 0)); return; }
        if (e.key === 'Enter') { e.preventDefault(); if (filtered[highlightIdx]) onSelect(filtered[highlightIdx]); return; }
        if (e.key === 'Escape') { e.preventDefault(); onClose(); }
    }
    return html`
        <div class="wt-picker" style=${style} ref=${containerRef}>
            <input class="wt-picker-search" type="text" placeholder="Filter branches..."
                ref=${inputRef}
                value=${query}
                onInput=${e => { setQuery(e.target.value); setHighlightIdx(0); }}
                onKeyDown=${handleKey} />
            <ul class="wt-picker-list" role="listbox">
                ${filtered.map((opt, i) => html`
                    <li key=${opt.branch}
                        class=${'wt-picker-option'
                            + (i === highlightIdx ? ' is-highlighted' : '')
                            + (opt.path === defaultWorktree || (!opt.path && !defaultWorktree) ? ' is-selected' : '')}
                        role="option"
                        onClick=${() => onSelect(opt)}
                        onMouseEnter=${() => setHighlightIdx(i)}
                    >${opt.branch}</li>
                `)}
                ${filtered.length === 0 && html`<li class="wt-picker-option is-empty">No matches</li>`}
            </ul>
        </div>
    `;
}

function DevIdleItem({ version, worktrees, defaultWorktree, setDefaultWorktree, onRecompile }) {
    const [pickerOpen, setPickerOpen] = useState(false);
    const [pickerBottom, setPickerBottom] = useState(0);
    const rowRef = useRef(null);
    const pickButtonRef = useRef(null);
    const options = [{ branch: 'main', path: null }, ...(worktrees || [])];
    const branch = defaultWorktree
        ? ((worktrees || []).find(w => w.path === defaultWorktree)?.branch ?? 'main')
        : 'main';
    function togglePicker() {
        if (!pickerOpen && rowRef.current) {
            setPickerBottom(window.innerHeight - rowRef.current.getBoundingClientRect().top);
        }
        setPickerOpen(v => !v);
    }
    return html`
        <div class="version-item is-dev wt-recompile-row" ref=${rowRef}>
            ${pickerOpen && html`<${WorktreePicker}
                style=${{ position: 'fixed', bottom: pickerBottom + 'px', left: 0, width: 'var(--size-sidebar)', zIndex: 'var(--z-popover)' }}
                options=${options}
                defaultWorktree=${defaultWorktree}
                onSelect=${opt => { setDefaultWorktree(opt.path); setPickerOpen(false); }}
                onClose=${() => setPickerOpen(false)}
                toggleButtonRef=${pickButtonRef}
            />`}
            <div class="wt-recompile-content" onClick=${onRecompile}>
                <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
                <span class="version-sub">↺ ${branch}</span>
            </div>
            <div class="wt-recompile-pick"
                ref=${pickButtonRef}
                role="button"
                tabIndex="0"
                aria-label="Pick recompile source"
                aria-expanded=${pickerOpen}
                onClick=${togglePicker}
            >≡</div>
        </div>
    `;
}

function renderDev(version, state, status, onAction, worktrees, defaultWorktree, setDefaultWorktree) {
    if (status === 'compiling') {
        const percent = clampPercent(state?.percent);
        const label = formatPhaseProgress(state?.phase, percent, 'Recompiling QoL Tray');
        return devProgress(version, label, percent, 'compiling');
    }
    if (status === 'downloading') {
        const percent = clampPercent(state?.percent);
        return devProgress(version, formatDownloadingProgress(percent), percent, '');
    }
    if (status === 'recompile_done') {
        return html`<div class="version-item is-dev update-done">
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Recompile complete</span>
        </div>`;
    }
    if (status === 'done') {
        return html`<div class="version-item is-dev update-done">
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Update complete</span>
        </div>`;
    }
    if (status === 'error') {
        const detail = state?.message ? `: ${state.message}` : '';
        return html`<div class="version-item is-dev" onClick=${() => onAction('dev-recompile')}>
            <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
            <span class="version-sub">Action failed${detail}. Click to retry</span>
        </div>`;
    }
    return html`<${DevIdleItem}
        version=${version}
        worktrees=${worktrees}
        defaultWorktree=${defaultWorktree}
        setDefaultWorktree=${setDefaultWorktree}
        onRecompile=${() => onAction('dev-recompile')}
    />`;
}

function renderStable(version, state, status, onAction) {
    if (status === 'downloading') {
        const percent = clampPercent(state?.percent);
        const displayPercent = visiblePercent(percent);
        return html`<div class="version-item is-downloading progress-track">
            <div class="progress-fill" style="--progress-scale: ${toProgressScale(displayPercent)}"></div>
            <span class="version-main">v${version}</span>
            <span class="version-sub">${formatDownloadingProgress(percent)}</span>
        </div>`;
    }
    if (status === 'done') {
        return html`<div class="version-item update-done">
            <span class="version-main">Restarting...</span>
            <span class="version-sub">v${version} installed</span>
        </div>`;
    }
    if (status === 'checking') {
        return html`<div class="version-item">
            <span class="version-main">v${version}</span>
            <span class="version-sub">Checking for updates...</span>
        </div>`;
    }
    if (status === 'available') {
        return html`<div class="version-item has-update" onClick=${() => onAction('self-update')}>
            <span class="version-main">v${state?.latest} available</span>
            <span class="version-sub">Click to update from v${version}</span>
        </div>`;
    }
    if (status === 'error') {
        return html`<div class="version-item" onClick=${() => onAction('check-update')}>
            <span class="version-main">v${version}</span>
            <span class="version-sub">Update failed. Click to retry</span>
        </div>`;
    }
    // idle / up-to-date
    return html`<div class="version-item" onClick=${() => onAction('check-update')}>
        <span class="version-main">v${version}</span>
        <span class="version-sub">Check for updates</span>
    </div>`;
}

function devProgress(version, label, percent, extraClass) {
    const displayPercent = visiblePercent(percent);
    return html`<div class="version-item is-dev is-downloading progress-track ${extraClass}">
        <div class="progress-fill" style="--progress-scale: ${toProgressScale(displayPercent)}"></div>
        <span class="version-main">v${version}<span class="version-tag">DEV</span></span>
        <span class="version-sub">${label}</span>
    </div>`;
}

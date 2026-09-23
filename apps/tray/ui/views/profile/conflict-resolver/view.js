import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef } from 'preact/hooks';
import { PageShell } from '../../../components/PageShell.js';
import { Surface } from '../../../lib/components/Surface.js';
import { Button } from '../../../lib/components/Button.js';
import { useRegisterViewKeyboard } from '../../../app/view-keyboard-context.js';
import { usePaletteContext } from '../../../palette/context.js';
import { useRegisterCommands } from '../../../palette/useRegisterCommands.js';
import { useResolver } from './use-resolver.js';
import { conflictKey, fieldDiff, formatValue, formatValueShort, leafKey, relativeTime } from './lib.js';

const VIEW_ID = 'profile-sync-conflicts';

const bootDevice = (typeof window !== 'undefined' && window.__QOL_BOOT__?.device) || null;
const THIS_DEVICE_NAME = bootDevice?.name?.trim() || 'This device';
const THIS_DEVICE_PLATFORM = bootDevice?.platform || '';

function dispatchEscape() {
    document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
}

export function ConflictResolverSubPage({ active, refreshSyncStatus }) {
    const onApplied = useCallback((result) => {
        refreshSyncStatus?.(result?.status);
        dispatchEscape();
    }, [refreshSyncStatus]);
    const resolver = useResolver({ active, onApplied });
    const { searchQuery } = usePaletteContext();

    const commands = useMemo(() => {
        if (!active || resolver.phase !== 'ready') return [];
        return [
            { id: 'profile-conflicts:keep-mine', label: `Conflict: keep ${THIS_DEVICE_NAME}`, run: () => resolver.pick('mine') },
            { id: 'profile-conflicts:take-remote', label: 'Conflict: take remote', run: () => resolver.pick('remote') },
            { id: 'profile-conflicts:next', label: 'Conflict: next', run: resolver.next },
            { id: 'profile-conflicts:prev', label: 'Conflict: previous', run: resolver.prev },
        ];
    }, [active, resolver.next, resolver.phase, resolver.pick, resolver.prev]);
    useRegisterCommands(VIEW_ID, commands);

    const handleKey = useCallback((event) => {
        if (!active) return;
        if (resolver.showConfirm) {
            if (event.key === 'Enter') {
                event.preventDefault();
                resolver.apply();
                return;
            }
            if (event.key === 'Escape') {
                event.preventDefault();
                resolver.backToSteps();
                return;
            }
            return;
        }
        if (resolver.phase !== 'ready') return;
        if (event.key === 'n') {
            event.preventDefault();
            if (resolver.index === resolver.total - 1 && resolver.ready) {
                resolver.openConfirm();
                return;
            }
            resolver.next();
            return;
        }
        if (event.key === 'p') {
            event.preventDefault();
            resolver.prev();
            return;
        }
    }, [active, resolver]);
    useRegisterViewKeyboard(VIEW_ID, handleKey, () => false);

    useEffect(() => {
        if (!active) return undefined;
        if (resolver.phase === 'empty') {
            const id = window.setTimeout(() => dispatchEscape(), 0);
            return () => window.clearTimeout(id);
        }
        return undefined;
    }, [active, resolver.phase]);

    if (!active) return null;
    if (resolver.phase === 'loading') {
        return html`<${PageShell} subtitle="Loading conflicts..." frameClassName="profile-conflicts-frame" />`;
    }
    if (resolver.phase === 'error') {
        return html`<${PageShell} subtitle="Failed to load conflicts" frameClassName="profile-conflicts-frame" />`;
    }
    if (resolver.phase === 'empty') {
        return html`<${PageShell} subtitle="No conflicts to resolve" frameClassName="profile-conflicts-frame" />`;
    }

    if (resolver.showConfirm) {
        return html`<${ConfirmCard} resolver=${resolver} />`;
    }

    return html`<${StepperCard} resolver=${resolver} searchQuery=${searchQuery} />`;
}

function StepperCard({ resolver, searchQuery }) {
    const { conflicts, current, index, picks, summary, total } = resolver;
    useSearchJump({ conflicts, index, moveTo: resolver.moveTo, searchQuery });
    const frameRef = useRef(null);
    useLayoutEffect(() => {
        const first = frameRef.current?.querySelector('.profile-conflicts-side[data-pick="mine"]');
        if (first instanceof HTMLElement) first.focus({ preventScroll: true });
    }, [index]);
    if (!current) return null;
    const subtitle = `Conflict ${index + 1} / ${total} - pick a side for each setting`;
    const choice = picks[index];
    const leaf = leafKey(current.key_path);
    const showPlugin = current.plugin && current.plugin !== leaf;

    return html`
        <${PageShell} subtitle=${subtitle} frameClassName="profile-conflicts-frame" frameRef=${frameRef}>
            <div class="profile-conflicts">
                <${ProgressDots} total=${total} index=${index} picks=${picks} />
                <div class="profile-conflicts-fieldhead">
                    ${showPlugin && html`<span class="profile-conflicts-plugin">${current.plugin}</span>`}
                    ${showPlugin && html`<span class="profile-conflicts-divider">${'·'}</span>`}
                    <span class="profile-conflicts-key">${leaf}</span>
                </div>
                <p class="profile-conflicts-file">${current.file}</p>
                <div class="profile-conflicts-sides">
                    <${SideCard} side="mine" label=${THIS_DEVICE_NAME} sublabel=${THIS_DEVICE_PLATFORM} conflict=${current} editedAt=${current.local_edited} picked=${choice === 'mine'} onActivate=${() => resolver.pick('mine')} />
                    <${SideCard} side="remote" label="Remote" conflict=${current} editedAt=${current.remote_edited} picked=${choice === 'remote'} onActivate=${() => resolver.pick('remote')} />
                </div>
                <p class="profile-conflicts-vs">- pick the value to keep -</p>
                <div class="profile-conflicts-footer">
                    <p class="profile-conflicts-summ">${summary.keptMine + summary.tookRemote} of ${total} resolved ${'·'} backup taken on apply</p>
                    <div class="profile-conflicts-nav">
                        <${Button} variant="btn-ghost" onActivate=${resolver.prev} disabled=${index === 0}>
                            <span aria-hidden="true">←</span> Prev
                        <//>
                        ${nextButton(resolver)}
                    </div>
                </div>
                <p class="profile-conflicts-kbd">
                    <kbd>←</kbd>/<kbd>→</kbd> move between sides   <kbd>enter</kbd> pick focused side   <kbd>n</kbd>/<kbd>p</kbd> next/prev conflict
                </p>
            </div>
        <//>
    `;
}

function nextButton(resolver) {
    const onLast = resolver.index === resolver.total - 1;
    if (onLast) {
        const label = resolver.ready ? 'Review' : 'Pick a side';
        return html`<${Button} variant="btn-primary" onActivate=${resolver.openConfirm} disabled=${!resolver.ready}>
            ${label} <span aria-hidden="true">→</span>
        <//>`;
    }
    return html`<${Button} variant="btn-primary" onActivate=${resolver.next} disabled=${resolver.picks[resolver.index] === null}>
        Next <span aria-hidden="true">→</span>
    <//>`;
}

function ProgressDots({ total, index, picks }) {
    const dots = [];
    for (let i = 0; i < total; i += 1) {
        const state = i === index ? 'active' : (picks[i] ? 'done' : 'pending');
        dots.push(html`<span key=${i} class="profile-conflicts-dot" data-state=${state}></span>`);
    }
    return html`<div class="profile-conflicts-dots">${dots}</div>`;
}

function SideCard({ side, label, sublabel, conflict, editedAt, picked, onActivate }) {
    return html`
        <${Surface} as="button"
            className=${`profile-conflicts-side profile-conflicts-side-${side}`}
            onActivate=${onActivate}
            data-pick=${side}
            data-picked=${picked ? 'true' : 'false'}
        >
            <span class="profile-conflicts-side-pin" aria-hidden="true"></span>
            <span class="profile-conflicts-side-who">
                ${label}
                ${sublabel && html`<span class="profile-conflicts-side-platform">${sublabel}</span>`}
            </span>
            <${SideValue} conflict=${conflict} side=${side} />
            <span class="profile-conflicts-side-meta">${relativeTime(editedAt)}</span>
        <//>
    `;
}

function SideValue({ conflict, side }) {
    const rows = fieldDiff(conflict.local, conflict.remote);
    if (!rows) {
        const value = side === 'mine' ? conflict.local : conflict.remote;
        return html`<span class="profile-conflicts-side-value">${formatValueShort(value, 64)}</span>`;
    }
    return html`
        <div class="profile-conflicts-side-fields">
            ${rows.map(row => html`
                <div class="profile-conflicts-field-row" key=${row.key}>
                    <span class="profile-conflicts-field-key">${row.key}</span>
                    <span class="profile-conflicts-field-val">${formatValueShort(side === 'mine' ? row.mine : row.remote)}</span>
                </div>
            `)}
        </div>
    `;
}

function ConfirmCard({ resolver }) {
    const { applying, conflicts, picks, summary, total } = resolver;
    return html`
        <${PageShell} subtitle=${`Review & apply - ${total} conflicts resolved`} frameClassName="profile-conflicts-frame">
            <div class="profile-conflicts profile-conflicts-confirm">
                <h2 class="profile-conflicts-confirm-title">Ready to merge your profile</h2>
                <p class="profile-conflicts-confirm-sub">Nothing is written until you press Apply. Both sides are backed up first.</p>
                <div class="profile-conflicts-totals">
                    <div class="profile-conflicts-stat">
                        <div class="profile-conflicts-stat-num">${summary.keptMine}</div>
                        <div class="profile-conflicts-stat-label">kept mine</div>
                    </div>
                    <div class="profile-conflicts-stat">
                        <div class="profile-conflicts-stat-num">${summary.tookRemote}</div>
                        <div class="profile-conflicts-stat-label">took remote</div>
                    </div>
                </div>
                <div class="profile-conflicts-confirm-rows" role="list">
                    ${conflicts.map((conflict, i) => html`
                        <div key=${conflictKey(conflict)} class="profile-conflicts-confirm-row" role="listitem">
                            <span class=${`profile-conflicts-confirm-swatch profile-conflicts-swatch-${picks[i]}`} aria-hidden="true"></span>
                            <span class="profile-conflicts-confirm-label">
                                ${conflict.plugin ? `${conflict.plugin} ${'·'} ${leafKey(conflict.key_path)}` : leafKey(conflict.key_path)}
                            </span>
                            <span class="profile-conflicts-confirm-value">
                                ${formatValue(picks[i] === 'mine' ? conflict.local : conflict.remote)}
                                <span class=${`profile-conflicts-confirm-side profile-conflicts-side-tag-${picks[i]}`}>
                                    (${picks[i] === 'mine' ? 'kept mine' : 'took remote'})
                                </span>
                            </span>
                        </div>
                    `)}
                </div>
                <p class="profile-conflicts-note">
                    On Apply: snapshot both sides → <code>sync/backups/&lt;ts&gt;-conflict.json</code>, write the merged profile, commit & push. One click in Backups undoes it.
                </p>
                <div class="profile-conflicts-confirm-actions">
                    <${Button} variant="btn-ghost" onActivate=${resolver.backToSteps} disabled=${applying}>
                        <span aria-hidden="true">←</span> Back to conflicts
                    <//>
                    <${Button} variant="btn-primary" onActivate=${resolver.apply} disabled=${applying || !resolver.ready}>
                        ${applying ? 'Applying...' : 'Apply & sync'}
                    <//>
                </div>
            </div>
        <//>
    `;
}

function matchesQuery(conflict, query) {
    if (!query) return true;
    const needle = query.toLowerCase();
    const haystack = `${conflict.plugin || ''} ${conflict.file} ${conflict.key_path}`.toLowerCase();
    return haystack.includes(needle);
}

function useSearchJump({ conflicts, index, moveTo, searchQuery }) {
    useEffect(() => {
        if (!searchQuery) return;
        const match = conflicts.findIndex(c => matchesQuery(c, searchQuery));
        if (match === -1 || match === index) return;
        moveTo(match - index);
    }, [conflicts, index, moveTo, searchQuery]);
}

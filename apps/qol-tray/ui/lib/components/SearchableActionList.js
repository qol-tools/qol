import { html } from '../html.js';
import { useState } from 'preact/hooks';
import { useInputSurface, Surface } from './Surface.js';
import { SurfaceContainer } from './SurfaceContainer.js';
import {
    ListGroup,
    ListRow,
    ListRowBody,
    ListRowHeader,
    ListRowText,
    ListRowTitle,
} from './ListRow.js';
import { useListSelection } from '../hooks/useListSelection.js';
import { filterSearchableItems, firstSearchableItemId } from '../searchable-action-list.js';
import { ActionMenu } from './ActionMenu.js';
import { Badge } from './StatusIndicators.js';

export function SearchableActionList({
    label,
    description,
    items = [],
    emptyMessage = 'No results.',
    placeholder = 'Search...',
    pendingId,
    pendingActionId,
    loading = false,
    error = null,
    searchable = true,
    onActivate,
    onAction,
    layout = 'comfortable',
    className,
    ...surfaceProps
}) {
    const [query, setQuery] = useState('');
    const selection = useListSelection();
    const visibleItems = filterSearchableItems(items, query);
    const inputSurface = useInputSurface({
        selectValue: 'search',
        onSelect: selection.select,
    });
    const cls = ['searchable-action-list', className].filter(Boolean).join(' ');
    const resultCount = query.trim()
        ? `${visibleItems.length}/${items.length}`
        : String(visibleItems.length);

    const handleInputKeyDown = (event) => {
        if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            event.currentTarget.closest('[data-searchable-action-list]')?.focus({ preventScroll: true });
            return;
        }
        if (event.key !== 'ArrowDown') return;
        const firstId = firstSearchableItemId(visibleItems);
        if (firstId == null) return;
        event.preventDefault();
        event.stopPropagation();
        event.currentTarget
            .closest('[data-surface-container]')
            ?.querySelector(`[data-searchable-item-id="${CSS.escape(String(firstId))}"]`)
            ?.focus({ preventScroll: true });
    };

    return html`
        <${Surface} className=${cls} data-searchable-action-list="" data-layout=${layout} ...${surfaceProps}>
            ${label && html`
                <div class="searchable-action-list-header">
                    <div class="searchable-action-list-label">${label}</div>
                    <span class="searchable-action-list-count" aria-label=${`${visibleItems.length} results`}>
                        ${resultCount}
                    </span>
                </div>
            `}
            ${description && html`<div class="searchable-action-list-description">${description}</div>`}
            <${SurfaceContainer} className="searchable-action-list-content">
                ${searchable && html`
                    <div class="searchable-action-list-search">
                        <span class="searchable-action-list-search-mark" aria-hidden="true">${'>'}</span>
                        <input ref=${inputSurface.ref} ...${inputSurface.attrs}
                            class="text-input searchable-action-list-input"
                            type="search"
                            value=${query}
                            aria-label=${label ? `Search ${label}` : 'Search results'}
                            placeholder=${placeholder}
                            onInput=${event => setQuery(event.currentTarget.value)}
                            onKeyDown=${handleInputKeyDown} />
                    </div>
                `}
                <${ListGroup} className="searchable-action-list-results" role="list"
                    onDeselect=${selection.deselect}>
                    ${visibleItems.map(item => html`
                        <${ListRow} key=${item.id}
                            role="listitem"
                            selectValue=${item.id}
                            selected=${selection.selected(item.id)}
                            onSelect=${selection.select}
                            onActivate=${item.disabled || !item.actionLabel
                                ? undefined
                                : () => onActivate?.(item)}
                            accent=${item.accent}
                            action=${item.actionLabel && html`
                                <div class="searchable-action-list-actions">
                                    <span class="searchable-action-list-action">
                                        ${pendingId === item.id ? 'Working...' : item.actionLabel}
                                    </span>
                                    ${!item.disabled && item.actions?.length > 1 && html`
                                        <${ActionMenu}
                                            label=${`Actions for ${item.label}`}
                                            actions=${item.actions}
                                            pendingId=${pendingId === item.id ? pendingActionId : null}
                                            onAction=${action => onAction?.(item, action)} />
                                    `}
                                </div>
                            `}
                            data-searchable-item-id=${item.id}
                            aria-disabled=${item.disabled ? 'true' : undefined}>
                            <${ListRowHeader}>
                                <${ListRowTitle}>${item.label}<//>
                                ${item.badge && html`
                                    <${Badge} className=${`searchable-action-list-badge tone-${item.badgeTone || item.accent || 'muted'}`}>
                                        ${item.badge}
                                    <//>
                                `}
                            <//>
                            ${item.description && html`
                                <${ListRowBody}>
                                    <${ListRowText}>${item.description}<//>
                                <//>
                            `}
                        <//>
                    `)}
                <//>
                ${loading && items.length === 0 && html`
                    <div class="searchable-action-list-empty">Loading...</div>
                `}
                ${!loading && visibleItems.length === 0 && html`
                    <div class="searchable-action-list-empty">
                        ${query.trim() ? 'No matching results.' : emptyMessage}
                    </div>
                `}
                ${error && html`<div class="field-action-error">${error}</div>`}
            <//>
        <//>
    `;
}

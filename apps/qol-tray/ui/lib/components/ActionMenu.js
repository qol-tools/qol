import { html } from '../html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { firstEnabledActionIndex, lastEnabledActionIndex, nextEnabledActionIndex } from '../action-menu.js';
import { useClickOutside } from '../hooks/useClickOutside.js';
import { Button } from './Button.js';
import { Surface } from './Surface.js';
import { SurfaceContainer } from './SurfaceContainer.js';

export function ActionMenu({ actions = [], label = 'More actions', pendingId, onAction, className }) {
    const [open, setOpen] = useState(false);
    const [highlightIndex, setHighlightIndex] = useState(() => firstEnabledActionIndex(actions));
    const containerRef = useRef(null);
    const cls = ['action-menu', className].filter(Boolean).join(' ');

    const close = useCallback(() => setOpen(false), []);
    useClickOutside(containerRef, open, close);

    const focusTrigger = useCallback(() => {
        containerRef.current?.querySelector('.action-menu-trigger')?.focus({ preventScroll: true });
    }, []);

    const openAt = useCallback((index) => {
        if (index < 0) return;
        setHighlightIndex(index);
        setOpen(true);
    }, []);

    const toggle = useCallback(() => {
        if (open) {
            close();
            return;
        }
        openAt(firstEnabledActionIndex(actions));
    }, [actions, close, open, openAt]);

    const run = useCallback((action) => {
        if (!action || action.disabled) return;
        close();
        onAction?.(action);
    }, [close, onAction]);

    const onTriggerKeyDown = useCallback((event) => {
        if (event.key !== 'ArrowDown' && event.key !== 'ArrowUp') return;
        event.preventDefault();
        event.stopPropagation();
        const index = event.key === 'ArrowUp'
            ? lastEnabledActionIndex(actions)
            : firstEnabledActionIndex(actions);
        openAt(index);
    }, [actions, openAt]);

    const onMenuKeyDown = useCallback((event) => {
        if (event.key === 'Tab') {
            close();
            return;
        }
        if (event.key === 'Escape') {
            event.preventDefault();
            event.stopPropagation();
            close();
            focusTrigger();
            return;
        }
        if (event.key === 'Home' || event.key === 'End') {
            event.preventDefault();
            event.stopPropagation();
            setHighlightIndex(event.key === 'Home'
                ? firstEnabledActionIndex(actions)
                : lastEnabledActionIndex(actions));
            return;
        }
        if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
            event.preventDefault();
            event.stopPropagation();
            setHighlightIndex(index => nextEnabledActionIndex(
                actions,
                index,
                event.key === 'ArrowDown' ? 1 : -1,
            ));
            return;
        }
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        run(actions[highlightIndex]);
        focusTrigger();
    }, [actions, close, focusTrigger, highlightIndex, run]);

    const onMenuBlur = useCallback((event) => {
        if (containerRef.current?.contains(event.relatedTarget)) return;
        close();
    }, [close]);

    useEffect(() => {
        if (!open) return;
        containerRef.current?.querySelector('.action-menu-list')?.focus({ preventScroll: true });
    }, [open]);

    useEffect(() => {
        if (!open) return;
        if (actions[highlightIndex] && !actions[highlightIndex].disabled) return;
        setHighlightIndex(firstEnabledActionIndex(actions));
    }, [actions, highlightIndex, open]);

    if (actions.length === 0) return null;
    return html`
        <div class=${cls} ref=${containerRef}>
            <${Button} variant="btn-ghost" small className="action-menu-trigger"
                aria-label=${label} aria-haspopup="menu" aria-expanded=${open ? 'true' : 'false'}
                onActivate=${toggle} onKeyDown=${onTriggerKeyDown} type="button">
                <span aria-hidden="true">${'⋯'}</span>
            <//>
            ${open && html`
                <${SurfaceContainer} className="action-menu-popover">
                    <div class="action-menu-list" role="menu" tabIndex="-1"
                        aria-label=${label} onKeyDown=${onMenuKeyDown} onBlur=${onMenuBlur}>
                        ${actions.map((action, index) => html`
                            <${Surface} as="button" type="button" role="menuitem"
                                key=${action.id || action.label}
                                className=${[
                                    'action-menu-item',
                                    index === highlightIndex && 'highlighted',
                                    action.tone && `tone-${action.tone}`,
                                ].filter(Boolean).join(' ')}
                                selected=${index === highlightIndex}
                                disabled=${action.disabled || pendingId != null}
                                onSelect=${() => setHighlightIndex(index)}
                                onMouseEnter=${() => setHighlightIndex(index)}
                                onActivate=${() => run(action)}>
                                ${pendingId === action.id ? 'Working...' : action.label}
                            <//>
                        `)}
                    </div>
                <//>
            `}
        </div>
    `;
}

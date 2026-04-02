import { html } from '../lib/html.js';
import { useState, useCallback, useRef, useEffect, useLayoutEffect } from 'preact/hooks';
import { SurfaceContainer } from './SurfaceContainer.js';
import { Surface, useInputSurface } from './Surface.js';
import { Button } from './Button.js';
import { useClickOutside } from '../hooks/useClickOutside.js';
import { useScrollFollow } from '../hooks/useScrollFollow.js';

export function CustomSelect({ value, options, labels, onChange, compact = false }) {
    const [open, setOpen] = useState(false);
    const [highlightIndex, setHighlightIndex] = useState(() => Math.max(0, options.indexOf(value)));
    const containerRef = useRef(null);
    const listSurface = useInputSurface();
    const [markerStyleState, setMarkerStyleState] = useState(hiddenMarkerStyle());

    const selectedLabel = (labels?.[value] || value) ?? '';

    const select = useCallback((opt) => {
        onChange(opt);
        setOpen(false);
    }, [onChange]);

    const onTriggerClick = useCallback(() => {
        if (!open) setHighlightIndex(Math.max(0, options.indexOf(value)));
        setOpen(!open);
    }, [open, options, value]);

    const onListKeyDown = useCallback((e) => {
        if (e.key === 'ArrowDown') {
            e.preventDefault();
            e.stopPropagation();
            setHighlightIndex(i => (i + 1) % options.length);
            return;
        }
        if (e.key === 'ArrowUp') {
            e.preventDefault();
            e.stopPropagation();
            setHighlightIndex(i => (i - 1 + options.length) % options.length);
            return;
        }
        if (e.key !== 'Enter' && e.key !== ' ' && e.key !== 'Escape' && e.key !== 'Tab') return;
        if (e.key === 'Enter' || e.key === ' ') select(options[highlightIndex]);
        if (e.key === 'Escape' || e.key === 'Tab') setOpen(false);
        e.preventDefault();
        e.stopPropagation();
        focusFieldLevel(containerRef.current);
    }, [options, highlightIndex, select]);

    const onListBlur = useCallback((e) => {
        if (containerRef.current?.contains(e.relatedTarget)) return;
        setOpen(false);
    }, []);

    const closeList = useCallback(() => setOpen(false), []);
    useClickOutside(containerRef, open, closeList);
    useScrollFollow(containerRef, open, highlightIndex, '.custom-select-option');

    useEffect(() => {
        if (!open) return;
        containerRef.current?.querySelector('.custom-select-list')?.focus();
    }, [open]);

    useEffect(() => {
        if (!open) return;
        const list = listSurface.ref.current;
        if (!(list instanceof HTMLElement)) return;
        const syncMarker = () => {
            const item = list.querySelector('.custom-select-option.highlighted');
            if (!(item instanceof HTMLElement)) return;
            setMarkerStyleState(settledMarkerStyle(list, item));
        };
        list.addEventListener('scroll', syncMarker, { passive: true });
        return () => list.removeEventListener('scroll', syncMarker);
    }, [open, highlightIndex, options.length, value]);

    useLayoutEffect(() => {
        if (!open) {
            setMarkerStyleState(hiddenMarkerStyle());
            return;
        }
        const list = listSurface.ref.current;
        const item = list?.querySelector('.custom-select-option.highlighted');
        if (!(list instanceof HTMLElement) || !(item instanceof HTMLElement)) {
            setMarkerStyleState(hiddenMarkerStyle());
            return;
        }
        setMarkerStyleState(settledMarkerStyle(list, item));
    }, [open, highlightIndex, options.length, value]);

    return html`
        <div class=${`custom-select${compact ? ' custom-select-compact' : ''}`} ref=${containerRef}>
            <${Button} variant="btn-dropdown" small=${compact} className="custom-select-trigger"
                onActivate=${onTriggerClick} type="button"
                aria-haspopup="listbox" aria-expanded=${open ? 'true' : 'false'}>
                <span class="custom-select-value">${selectedLabel}</span>
                <span class="custom-select-arrow">${'\u25BE'}</span>
            <//>
            ${open && html`
                <${SurfaceContainer} className="custom-select-popover">
                    <div class="custom-select-list" ref=${listSurface.ref} tabIndex="-1" onKeyDown=${onListKeyDown} onBlur=${onListBlur} ...${listSurface.attrs}>
                        ${options.map((opt, i) => html`
                            <${Surface} key=${opt}
                                 className="custom-select-option ${opt === value ? 'selected' : ''} ${i === highlightIndex ? 'highlighted' : ''}"
                                 data-selected-surface-priority="10"
                                 selected=${i === highlightIndex}
                                 onActivate=${() => select(opt)}
                                 onMouseEnter=${() => setHighlightIndex(i)}>
                                ${labels?.[opt] || opt}
                            <//>
                        `)}
                    <//>
                    <div class="custom-select-active-marker"
                        aria-hidden="true" style=${markerStyleState} />
                <//>
            `}
        </div>
    `;
}

function focusFieldLevel(el) {
    const field = el?.closest('[data-plugin-config-field-id]');
    if (field) { field.focus(); return; }
    const surface = el?.closest('[data-selected-surface]');
    if (surface) { surface.focus(); return; }
    el?.querySelector('.custom-select-trigger')?.focus();
}

function hiddenMarkerStyle() {
    return {
        opacity: 0,
        height: 'var(--custom-select-option-height, 36px)',
        transform: 'translate(0px, 0px)',
    };
}

function settledMarkerStyle(list, item) {
    return {
        opacity: 1,
        height: 'var(--custom-select-option-height, 36px)',
        transform: `translate(0px, ${item.offsetTop - list.scrollTop}px)`,
    };
}
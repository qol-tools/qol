import { html } from '../../../lib/html.js';
import { useState, useCallback, useRef, useEffect, useLayoutEffect } from 'preact/hooks';

export function CustomSelect({ value, options, labels, onChange }) {
    const [open, setOpen] = useState(false);
    const [highlightIndex, setHighlightIndex] = useState(() => Math.max(0, options.indexOf(value)));
    const containerRef = useRef(null);
    const listRef = useRef(null);
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

    useEffect(() => {
        if (!open) return;
        const onPointerDown = (e) => {
            if (containerRef.current?.contains(e.target)) return;
            setOpen(false);
        };
        document.addEventListener('pointerdown', onPointerDown);
        return () => document.removeEventListener('pointerdown', onPointerDown);
    }, [open]);

    useEffect(() => {
        if (!open) return;
        containerRef.current?.querySelector('.custom-select-list')?.focus();
    }, [open]);

    useLayoutEffect(() => {
        if (!open) return;
        const items = containerRef.current?.querySelectorAll('.custom-select-option');
        items?.[highlightIndex]?.scrollIntoView({ block: 'nearest' });
    }, [open, highlightIndex]);

    useEffect(() => {
        if (!open) return;
        const list = listRef.current;
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
        const list = listRef.current;
        const item = list?.querySelector('.custom-select-option.highlighted');
        if (!(list instanceof HTMLElement) || !(item instanceof HTMLElement)) {
            setMarkerStyleState(hiddenMarkerStyle());
            return;
        }
        setMarkerStyleState(settledMarkerStyle(list, item));
    }, [open, highlightIndex, options.length, value]);

    return html`
        <div class="custom-select" ref=${containerRef}>
            <button type="button" class="custom-select-trigger" data-focus-chrome="parent" data-wedge-root="" onClick=${onTriggerClick}>
                <span class="custom-select-value">${selectedLabel}</span>
                <span class="custom-select-arrow">\u25BE</span>
            </button>
            ${open && html`
                <div class="custom-select-popover">
                    <div class="custom-select-list" ref=${listRef} tabIndex="-1" onKeyDown=${onListKeyDown} onBlur=${onListBlur}>
                        ${options.map((opt, i) => html`
                            <div key=${opt}
                                 class="custom-select-option ${opt === value ? 'selected' : ''} ${i === highlightIndex ? 'highlighted' : ''}"
                                 data-selected-surface=""
                                 data-selected-surface-priority="10"
                                 data-selected=${i === highlightIndex ? 'true' : 'false'}
                                 onClick=${() => select(opt)}
                                 onMouseEnter=${() => setHighlightIndex(i)}>
                                ${labels?.[opt] || opt}
                            </div>
                        `)}
                    </div>
                    <div class="custom-select-active-marker"
                        aria-hidden="true" style=${markerStyleState} />
                </div>
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
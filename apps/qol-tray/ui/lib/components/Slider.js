import { html } from '../html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { useSurface } from './Surface.js';
import { SURFACE_CONTROL_COMMIT, finishSurfaceFocusTarget } from '../surface-focus-target.js';

const DEFAULT_MIN = 0;
const DEFAULT_MAX = 100;
const DEFAULT_STEP = 1;

export function Slider({
    value,
    min = DEFAULT_MIN,
    max = DEFAULT_MAX,
    step = DEFAULT_STEP,
    label = '',
    description = '',
    unit = '',
    disabled = false,
    className = '',
    formatter,
    onInput,
    onCommit,
    onActiveChange,
}) {
    const initial = normalizeValue(value, min, max, step);
    const [liveValue, setLiveValue] = useState(initial);
    const [active, setActive] = useState(false);
    const [dragging, setDragging] = useState(false);
    const trackRef = useRef(null);
    const thumbRef = useRef(null);
    const trackingRef = useRef(false);
    const liveValueRef = useRef(initial);
    const committedRef = useRef(initial);

    useEffect(() => {
        const next = normalizeValue(value, min, max, step);
        setLiveValue(next);
        liveValueRef.current = next;
        committedRef.current = next;
        syncTrackPosition(trackRef.current, next, min, max);
        nudgeSelectedSurface(thumbRef.current);
    }, [value, min, max, step]);

    const setActiveState = useCallback((next) => {
        setActive(next);
        onActiveChange?.(next);
    }, [onActiveChange]);

    const updateLiveValue = useCallback((next, { snap = true } = {}) => {
        const normalized = normalizeValue(next, min, max, step, snap);
        const callbackValue = snap ? normalized : normalizeValue(normalized, min, max, step);
        setLiveValue(normalized);
        liveValueRef.current = normalized;
        syncTrackPosition(trackRef.current, normalized, min, max);
        nudgeSelectedSurface(thumbRef.current);
        onInput?.(callbackValue);
    }, [min, max, step, onInput]);

    const commitLiveValue = useCallback(() => {
        const next = normalizeValue(liveValueRef.current, min, max, step);
        setLiveValue(next);
        liveValueRef.current = next;
        syncTrackPosition(trackRef.current, next, min, max);
        nudgeSelectedSurface(thumbRef.current);
        if (Object.is(next, committedRef.current)) {
            return;
        }
        committedRef.current = next;
        onCommit?.(next);
    }, [min, max, step, onCommit]);

    useEffect(() => {
        const thumb = thumbRef.current;
        if (!thumb) {
            return;
        }
        thumb.addEventListener('slider-commit', commitLiveValue);
        thumb.addEventListener(SURFACE_CONTROL_COMMIT, commitLiveValue);
        return () => {
            thumb.removeEventListener('slider-commit', commitLiveValue);
            thumb.removeEventListener(SURFACE_CONTROL_COMMIT, commitLiveValue);
        };
    }, [commitLiveValue]);

    const updateFromPointer = useCallback((event) => {
        const track = trackRef.current;
        if (!track || disabled) {
            return;
        }
        const rect = track.getBoundingClientRect();
        if (rect.width <= 0) {
            return;
        }
        const ratio = clamp((event.clientX - rect.left) / rect.width, 0, 1);
        updateLiveValue(min + ratio * (max - min), { snap: false });
    }, [disabled, min, max, updateLiveValue]);

    const onPointerDown = useCallback((event) => {
        if (disabled) {
            return;
        }
        trackingRef.current = true;
        setDragging(true);
        trackRef.current?.setPointerCapture?.(event.pointerId);
        setActiveState(true);
        updateFromPointer(event);
    }, [disabled, setActiveState, updateFromPointer]);

    const finishPointer = useCallback((event) => {
        if (!trackingRef.current) {
            return;
        }
        trackingRef.current = false;
        setDragging(false);
        trackRef.current?.releasePointerCapture?.(event.pointerId);
        commitLiveValue();
        if (document.activeElement !== thumbRef.current) {
            setActiveState(false);
        }
    }, [commitLiveValue, setActiveState]);

    const onThumbFocus = useCallback(() => {
        if (!disabled) {
            setActiveState(true);
        }
    }, [disabled, setActiveState]);

    const onThumbBlur = useCallback(() => {
        setActiveState(false);
        commitLiveValue();
    }, [setActiveState, commitLiveValue]);

    const onThumbKeyDown = useCallback((event) => {
        if (disabled) {
            return;
        }
        if (event.key === 'Enter' || event.key === ' ') {
            event.preventDefault();
            event.stopPropagation();
            finishSurfaceFocusTarget(event.currentTarget);
            return;
        }
        const next = keyValue(event.key, event.shiftKey, liveValueRef.current, min, max, step);
        if (next == null) {
            return;
        }
        event.preventDefault();
        event.stopPropagation();
        updateLiveValue(next);
    }, [disabled, min, max, step, updateLiveValue]);

    const thumbLeft = pct(liveValue, min, max);
    const sliderStyle = `--slider-progress:${thumbLeft / 100};--slider-position:${thumbLeft}%`;
    const display = formatter ? formatter(liveValue) : formatValue(liveValue, step);
    const cls = ['slider-control', disabled && 'is-disabled', className].filter(Boolean).join(' ');
    const trackClass = ['slider-track', dragging && 'is-dragging'].filter(Boolean).join(' ');
    const { attrs: thumbAttrs } = useSurface({ selected: active, priority: 10 });
    const { attrs: controlAttrs } = useSurface();
    const focusTargetAttrs = disabled ? {} : { 'data-surface-focus-target': '' };

    return html`
        <div class=${cls} aria-label=${label || 'Slider'} ...${disabled ? {} : controlAttrs}>
            ${label && html`
                <div class="slider-header">
                    <label class="slider-label">${label}</label>
                    <span class="slider-value">${display}${unit}</span>
                </div>
            `}
            ${description && html`<div class="slider-description">${description}</div>`}
            <div ref=${trackRef} class=${trackClass} style=${sliderStyle}
                onPointerDown=${onPointerDown}
                onPointerMove=${event => trackingRef.current && updateFromPointer(event)}
                onPointerUp=${finishPointer}
                onPointerCancel=${finishPointer}>
                <div class="slider-fill"></div>
                <div class="slider-thumb" style=${disabled ? 'opacity:0.4' : ''}></div>
                <div ref=${thumbRef} class="slider-thumb-target" data-slider-thumb=""
                    ...${focusTargetAttrs}
                    role="slider"
                    aria-label=${label || 'Slider'}
                    aria-valuemin=${min}
                    aria-valuemax=${max}
                    aria-valuenow=${liveValue}
                    aria-valuetext=${`${display}${unit}`}
                    aria-disabled=${disabled ? 'true' : undefined}
                    ...${disabled ? {} : thumbAttrs}
                    onFocus=${onThumbFocus}
                    onBlur=${onThumbBlur}
                    onKeyDown=${onThumbKeyDown}></div>
            </div>
        </div>
    `;
}

function keyValue(key, shiftKey, value, min, max, step) {
    const delta = step * (shiftKey ? 5 : 1);
    if (key === 'ArrowLeft' || key === 'ArrowDown') {
        return value - delta;
    }
    if (key === 'ArrowRight' || key === 'ArrowUp') {
        return value + delta;
    }
    if (key === 'Home') {
        return min;
    }
    if (key === 'End') {
        return max;
    }
    return null;
}

function normalizeValue(value, min, max, step, snap = true) {
    const raw = typeof value === 'number' && Number.isFinite(value) ? value : min;
    if (!snap) {
        return clamp(raw, min, max);
    }
    const snapped = snapToStep(raw, min, step);
    return clamp(snapped, min, max);
}

function snapToStep(value, min, step) {
    if (!step || step <= 0) {
        return value;
    }
    const snapped = min + Math.round((value - min) / step) * step;
    return parseFloat(snapped.toFixed(6));
}

function clamp(value, min, max) {
    return Math.max(min, Math.min(max, value));
}

function pct(value, min, max) {
    if (max === min) {
        return 0;
    }
    return clamp(((value - min) / (max - min)) * 100, 0, 100);
}

function syncTrackPosition(track, value, min, max) {
    if (typeof HTMLElement === 'undefined' || !(track instanceof HTMLElement)) {
        return;
    }
    const left = pct(value, min, max);
    track.style.setProperty('--slider-progress', `${left / 100}`);
    track.style.setProperty('--slider-position', `${left}%`);
}

function nudgeSelectedSurface(el) {
    if (typeof HTMLElement === 'undefined' || !(el instanceof HTMLElement)) {
        return;
    }
    if (el.hasAttribute('data-selected-surface-motion')) {
        el.removeAttribute('data-selected-surface-motion');
    } else {
        el.setAttribute('data-selected-surface-motion', 'teleport');
    }
}

function formatValue(value, step) {
    if (Number.isInteger(value)) {
        return `${value}`;
    }
    return value.toFixed(decimalPlaces(step));
}

function decimalPlaces(value) {
    const text = `${value}`;
    const decimal = text.indexOf('.');
    if (decimal === -1) {
        return 0;
    }
    return text.length - decimal - 1;
}

import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { openColorStream, closeColorStream, streamBrightness } from './color-stream.js';

const DEFAULT_MIN = 0;
const DEFAULT_MAX = 100;

export function SliderField({ field }) {
    const ctx = usePluginConfigContext();
    const hasStream = !!field.stream;
    const { dispatch: sendAction } = useDispatchAction(ctx.pluginId, field.action || null);
    const stored = ctx.getFieldValue(field);
    const min = field.number?.min ?? field.min ?? DEFAULT_MIN;
    const max = field.number?.max ?? field.max ?? DEFAULT_MAX;
    const step = field.number?.step ?? field.step ?? 1;
    const initial = typeof stored === 'number' ? stored : min;

    const [value, setValue] = useState(initial);
    const [active, setActive] = useState(false);
    const trackRef = useRef(null);
    const thumbTargetRef = useRef(null);
    const trackingRef = useRef(false);
    const liveValueRef = useRef(initial);

    useEffect(() => {
        if (typeof stored === 'number') {
            setValue(stored);
            liveValueRef.current = stored;
        }
    }, [stored]);

    const commit = useCallback((v) => {
        ctx.setFieldValue(field, v);
        ctx.saveNow().then(() => {
            if (sendAction) sendAction().catch(() => {});
        });
    }, [ctx, field, sendAction]);

    useEffect(() => {
        const el = thumbTargetRef.current;
        if (!el) return;
        const onCommit = () => commit(liveValueRef.current);
        el.addEventListener('slider-commit', onCommit);
        return () => el.removeEventListener('slider-commit', onCommit);
    }, [commit]);

    useEffect(() => {
        const thumb = thumbTargetRef.current;
        if (!thumb) return;
        const dirs = new Set();
        let anim = null;
        let shift = false;
        const range = max - min;
        const SPEED = range / 1;
        const SPEED_SHIFT = range / 0.35;

        function tick(now) {
            if (!anim) return;
            const dt = Math.min((now - anim.t) / 1000, 0.05);
            anim.t = now;
            const delta = (shift ? SPEED_SHIFT : SPEED) * dt;
            let v = liveValueRef.current;
            if (dirs.has('left')) v -= delta;
            if (dirs.has('right')) v += delta;
            v = Math.max(min, Math.min(max, v));
            const rounded = step >= 1 ? Math.round(v) : Math.round(v / step) * step;
            setValue(rounded);
            liveValueRef.current = rounded;
            if (hasStream) streamValue(field, rounded, ctx);
            thumb.style.left = pct(rounded, min, max) + '%';
            nudgeWedge(thumb);
            anim.frame = requestAnimationFrame(tick);
        }

        function onDown(e) {
            const map = { ArrowLeft: 'left', ArrowRight: 'right' };
            const dir = map[e.key];
            if (!dir) return;
            e.preventDefault();
            shift = e.shiftKey;
            dirs.add(dir);
            if (!anim) {
                anim = { t: performance.now(), frame: 0 };
                anim.frame = requestAnimationFrame(tick);
            }
        }

        function onUp(e) {
            const map = { ArrowLeft: 'left', ArrowRight: 'right' };
            if (!map[e.key]) return;
            dirs.delete(map[e.key]);
            if (dirs.size === 0 && anim) {
                cancelAnimationFrame(anim.frame);
                anim = null;
            }
        }

        thumb.addEventListener('keydown', onDown);
        thumb.addEventListener('keyup', onUp);
        return () => {
            thumb.removeEventListener('keydown', onDown);
            thumb.removeEventListener('keyup', onUp);
            if (anim) cancelAnimationFrame(anim.frame);
        };
    }, [hasStream, min, max, step, field]);

    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);

    const onThumbFocus = useCallback(() => {
        setActive(true);
        if (hasStream) openColorStream();
    }, [hasStream]);

    const onThumbBlur = useCallback(() => {
        setActive(false);
        if (hasStream) closeColorStream();
        ctx.setFieldValue(field, liveValueRef.current);
        ctx.save();
    }, [hasStream, ctx, field]);

    const onPointer = useCallback((e) => {
        const rect = trackRef.current.getBoundingClientRect();
        const ratio = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
        const raw = min + ratio * (max - min);
        const rounded = step >= 1 ? Math.round(raw) : Math.round(raw / step) * step;
        setValue(rounded);
        liveValueRef.current = rounded;
        if (hasStream) streamValue(field, rounded);
    }, [min, max, step, hasStream, field]);

    const onPointerDown = useCallback((e) => {
        trackingRef.current = true;
        trackRef.current.setPointerCapture(e.pointerId);
        if (hasStream) openColorStream();
        onPointer(e);
    }, [onPointer, hasStream]);

    const onPointerMove = useCallback((e) => {
        if (!trackingRef.current) return;
        onPointer(e);
    }, [onPointer]);

    const onPointerUp = useCallback(() => {
        if (!trackingRef.current) return;
        trackingRef.current = false;
        if (hasStream) closeColorStream();
        commit(liveValueRef.current);
    }, [commit, hasStream]);

    const gated = ctx.isRuntimeDisabled && field.stream;
    const thumbLeft = pct(value, min, max);
    const unit = field.unit || '';

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, `field-group field-slider${gated ? ' field-gated' : ''}`)}
            onMouseDown=${onSelect} onFocus=${onSelect}>
            <div class="slider-header">
                <label class="slider-label">${field.label}</label>
                <span class="slider-value">${value}${unit}</span>
            </div>
            <div ref=${trackRef} class="slider-track"
                onPointerDown=${gated ? undefined : onPointerDown}
                onPointerMove=${gated ? undefined : onPointerMove}
                onPointerUp=${gated ? undefined : onPointerUp}
                onPointerCancel=${gated ? undefined : onPointerUp}>
                <div class="slider-fill" style="width:${thumbLeft}%"></div>
                <div class="slider-thumb" style="left:${thumbLeft}%;${gated ? 'opacity:0.4' : ''}"></div>
                <div ref=${thumbTargetRef} class="slider-thumb-target" data-slider-thumb=""
                    data-selected-surface="" data-selected=${active ? 'true' : 'false'}
                    data-selected-surface-priority="10"
                    tabIndex="-1"
                    onFocus=${onThumbFocus} onBlur=${onThumbBlur}
                    style="left:${thumbLeft}%"></div>
            </div>
        </div>
    `;
}

function pct(value, min, max) {
    if (max === min) return 0;
    return Math.max(0, Math.min(100, ((value - min) / (max - min)) * 100));
}

function streamValue(field, value, ctx) {
    if (field.config_key === 'live_brightness') {
        const colorHex = (ctx.state?.config?.live_color_hex || '#ffffff').replace(/^#/, '');
        streamBrightness(value, colorHex);
    }
}

function nudgeWedge(el) {
    if (!el) return;
    if (el.hasAttribute('data-selected-surface-motion')) {
        el.removeAttribute('data-selected-surface-motion');
    } else {
        el.setAttribute('data-selected-surface-motion', 'teleport');
    }
}

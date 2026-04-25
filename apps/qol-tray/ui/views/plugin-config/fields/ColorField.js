import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { useSurface } from '../../../lib/components/Surface.js';
import { hueComponents, hueSatToHex, hexToHueSat } from './color-math.js';
import { openColorStream, closeColorStream, streamColorHex } from './color-stream.js';

const DISC_SIZE = 200;
const THUMB_R = 8;
const DISC_RADIUS = DISC_SIZE / 2 - 1;

export function ColorField({ field }) {
    const ctx = usePluginConfigContext();
    const hasStream = !!field.stream;
    const { dispatch: sendColor } = useDispatchAction(ctx.pluginId, hasStream ? 'set_color_main' : null);
    const stored = ctx.getFieldValue(field);
    const raw = typeof stored === 'string' ? stored.replace(/^#/, '') : 'ffffff';
    const initial = hexToHueSat(raw);

    const [hue, setHue] = useState(initial.hue);
    const [sat, setSat] = useState(initial.saturation);
    const [thumbActive, setThumbActive] = useState(false);
    const canvasRef = useRef(null);
    const offscreenRef = useRef(null);
    const outerRef = useRef(null);
    const thumbRef = useRef(null);
    const trackingRef = useRef(null);
    const liveHueRef = useRef(initial.hue);
    const liveSatRef = useRef(initial.saturation);

    useEffect(() => {
        const parsed = hexToHueSat(raw);
        setHue(parsed.hue);
        setSat(parsed.saturation);
        liveHueRef.current = parsed.hue;
        liveSatRef.current = parsed.saturation;
    }, [raw]);

    useEffect(() => {
        if (!offscreenRef.current) {
            offscreenRef.current = document.createElement('canvas');
            offscreenRef.current.width = DISC_SIZE;
            offscreenRef.current.height = DISC_SIZE;
            prerenderDisc(offscreenRef.current);
        }
        drawDisc(canvasRef.current, offscreenRef.current, hue, sat, !thumbActive);
    }, [hue, sat, thumbActive]);

    const commitColor = useCallback((h, s) => {
        const hex = '#' + hueSatToHex(h, s);
        ctx.setFieldValue(field, hex);
        ctx.saveNow().then(() => {
            if (sendColor) sendColor().catch(() => {});
        });
    }, [ctx, field, sendColor]);

    useEffect(() => {
        const el = outerRef.current;
        if (!el) return;
        const onCommit = () => commitColor(liveHueRef.current, liveSatRef.current);
        el.addEventListener('color-commit', onCommit);
        return () => el.removeEventListener('color-commit', onCommit);
    }, [commitColor]);

    useEffect(() => {
        const thumb = thumbRef.current;
        if (!thumb) return;
        const dirs = new Set();
        let anim = null;
        let shift = false;
        const SPEED = 120;
        const SPEED_SHIFT = 280;

        function tick(now) {
            if (!anim) return;
            const dt = Math.min((now - anim.t) / 1000, 0.05);
            anim.t = now;
            const px = (shift ? SPEED_SHIFT : SPEED) * dt;
            const h = liveHueRef.current;
            const s = liveSatRef.current;
            const angle = h * Math.PI / 180;
            const dist = s * DISC_RADIUS;
            let x = dist * Math.cos(angle);
            let y = dist * Math.sin(angle);
            if (dirs.has('left')) x -= px;
            if (dirs.has('right')) x += px;
            if (dirs.has('up')) y -= px;
            if (dirs.has('down')) y += px;
            const newDist = Math.min(Math.sqrt(x * x + y * y), DISC_RADIUS);
            const newH = ((Math.atan2(y, x) * 180 / Math.PI) + 360) % 360;
            const newS = newDist / DISC_RADIUS;
            setHue(newH);
            setSat(newS);
            liveHueRef.current = newH;
            liveSatRef.current = newS;
            if (hasStream) streamColorHex(hueSatToHex(newH, newS));
            const center = DISC_SIZE / 2;
            const tx = center + newS * DISC_RADIUS * Math.cos(newH * Math.PI / 180);
            const ty = center + newS * DISC_RADIUS * Math.sin(newH * Math.PI / 180);
            thumb.style.left = tx + 'px';
            thumb.style.top = ty + 'px';
            nudgeWedge(thumb);
            anim.frame = requestAnimationFrame(tick);
        }

        function onDown(e) {
            const map = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' };
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
            const map = { ArrowLeft: 'left', ArrowRight: 'right', ArrowUp: 'up', ArrowDown: 'down' };
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
    }, [hasStream]);

    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);

    const onThumbFocus = useCallback(() => {
        setThumbActive(true);
        if (hasStream) openColorStream();
    }, [hasStream]);

    const onThumbBlur = useCallback(() => {
        setThumbActive(false);
        if (hasStream) closeColorStream();
        const hex = '#' + hueSatToHex(liveHueRef.current, liveSatRef.current);
        ctx.setFieldValue(field, hex);
        ctx.save();
    }, [hasStream, ctx, field]);

    const onDiscPointer = useCallback((e) => {
        const rect = canvasRef.current.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;
        const center = DISC_SIZE / 2;
        const dx = x - center;
        const dy = y - center;
        const dist = Math.min(Math.sqrt(dx * dx + dy * dy), DISC_RADIUS);
        const h = ((Math.atan2(dy, dx) * 180 / Math.PI) + 360) % 360;
        const s = dist / DISC_RADIUS;
        setHue(h);
        setSat(s);
        liveHueRef.current = h;
        liveSatRef.current = s;
        if (hasStream) streamColorHex(hueSatToHex(h, s));
        return { h, s };
    }, [hasStream]);

    const onDiscDown = useCallback((e) => {
        const rect = canvasRef.current.getBoundingClientRect();
        const cx = DISC_SIZE / 2;
        if (Math.sqrt((e.clientX - rect.left - cx) ** 2 + (e.clientY - rect.top - cx) ** 2) > cx) return;
        trackingRef.current = 'disc';
        canvasRef.current.setPointerCapture(e.pointerId);
        if (hasStream) openColorStream();
        onDiscPointer(e);
    }, [onDiscPointer, hasStream]);

    const onDiscMove = useCallback((e) => {
        if (trackingRef.current !== 'disc') return;
        onDiscPointer(e);
    }, [onDiscPointer]);

    const onDiscUp = useCallback(() => {
        if (trackingRef.current !== 'disc') return;
        trackingRef.current = null;
        if (hasStream) closeColorStream();
        commitColor(liveHueRef.current, liveSatRef.current);
    }, [commitColor, hasStream]);

    const gated = ctx.isRuntimeDisabled && field.stream;
    const center = DISC_SIZE / 2;
    const thumbAngle = hue * Math.PI / 180;
    const thumbDist = sat * DISC_RADIUS;
    const thumbX = center + thumbDist * Math.cos(thumbAngle);
    const thumbY = center + thumbDist * Math.sin(thumbAngle);
    const { attrs: thumbAttrs } = useSurface({ selected: thumbActive, priority: 10 });

    return html`
        <div ref=${outerRef} ...${fieldSurfaceAttrs(field, ctx, `field-group field-color${gated ? ' field-gated' : ''}`)}
            onMouseDown=${onSelect} onFocus=${onSelect}>
            <label class="field-color-label">${field.label}</label>
            <div class="hue-wheel-container${gated ? ' hue-wheel-disabled' : ''}">
                <div class="hue-disc-wrap">
                    <canvas ref=${canvasRef} class="hue-disc" width=${DISC_SIZE} height=${DISC_SIZE}
                        onPointerDown=${gated ? undefined : onDiscDown}
                        onPointerMove=${gated ? undefined : onDiscMove}
                        onPointerUp=${gated ? undefined : onDiscUp}
                        onPointerCancel=${gated ? undefined : onDiscUp}
                        style=${gated ? 'pointer-events:none;opacity:0.4' : ''} />
                    <div ref=${thumbRef} class="color-thumb-target" data-color-thumb=""
                        ...${thumbAttrs}
                        onFocus=${onThumbFocus} onBlur=${onThumbBlur}
                        style="left:${thumbX}px;top:${thumbY}px" />
                </div>
            </div>
        </div>
    `;
}

function nudgeWedge(el) {
    if (!el) return;
    if (el.hasAttribute('data-selected-surface-motion')) {
        el.removeAttribute('data-selected-surface-motion');
    } else {
        el.setAttribute('data-selected-surface-motion', 'teleport');
    }
}

function prerenderDisc(canvas) {
    const ctx = canvas.getContext('2d');
    const w = canvas.width;
    const center = w / 2;
    const radius = center - 1;
    const imageData = ctx.createImageData(w, w);
    const d = imageData.data;
    for (let py = 0; py < w; py++) {
        for (let px = 0; px < w; px++) {
            const dx = px - center;
            const dy = py - center;
            const dist = Math.sqrt(dx * dx + dy * dy);
            if (dist > radius) continue;
            const angle = Math.atan2(dy, dx);
            const h = ((angle * 180 / Math.PI) + 360) % 360;
            const s = dist / radius;
            const [hr, hg, hb] = hueComponents(h);
            const i = (py * w + px) * 4;
            d[i]     = Math.round((1 - s + s * hr) * 255);
            d[i + 1] = Math.round((1 - s + s * hg) * 255);
            d[i + 2] = Math.round((1 - s + s * hb) * 255);
            d[i + 3] = 255;
        }
    }
    ctx.putImageData(imageData, 0, 0);
}

function drawDisc(canvas, offscreen, hue, sat, showThumb) {
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    const center = DISC_SIZE / 2;
    const radius = center - 1;
    ctx.clearRect(0, 0, DISC_SIZE, DISC_SIZE);
    ctx.drawImage(offscreen, 0, 0);
    if (!showThumb) return;
    const angle = hue * Math.PI / 180;
    const dist = sat * radius;
    const tx = center + dist * Math.cos(angle);
    const ty = center + dist * Math.sin(angle);
    ctx.save();
    ctx.beginPath();
    ctx.arc(tx, ty, THUMB_R, 0, Math.PI * 2);
    ctx.fillStyle = '#' + hueSatToHex(hue, sat);
    ctx.fill();
    ctx.lineWidth = 3;
    ctx.strokeStyle = 'white';
    ctx.shadowColor = 'rgba(0,0,0,0.5)';
    ctx.shadowBlur = 6;
    ctx.stroke();
    ctx.restore();
}

import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { fieldSurfaceAttrs } from '../field-map.js';

const DISC_SIZE = 200;
const THUMB_R = 8;

export function ColorField({ field }) {
    const ctx = usePluginConfigContext();
    const stored = ctx.getFieldValue(field);
    const raw = typeof stored === 'string' ? stored.replace(/^#/, '') : 'ffffff';
    const initial = hexToHueSat(raw);

    const [hue, setHue] = useState(initial.hue);
    const [sat, setSat] = useState(initial.saturation);
    const [brightness, setBrightness] = useState(100);
    const canvasRef = useRef(null);
    const offscreenRef = useRef(null);
    const trackingRef = useRef(null);

    useEffect(() => {
        const parsed = hexToHueSat(raw);
        setHue(parsed.hue);
        setSat(parsed.saturation);
    }, [raw]);

    useEffect(() => {
        if (!offscreenRef.current) {
            offscreenRef.current = document.createElement('canvas');
            offscreenRef.current.width = DISC_SIZE;
            offscreenRef.current.height = DISC_SIZE;
            prerenderDisc(offscreenRef.current);
        }
        drawDisc(canvasRef.current, offscreenRef.current, hue, sat);
    }, [hue, sat]);

    const commit = useCallback((h, s, b) => {
        const hex = '#' + hueSatToHex(h, s);
        ctx.setFieldValue(field, hex);
        ctx.save();
    }, [ctx, field]);

    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);

    const onDiscPointer = useCallback((e) => {
        const rect = canvasRef.current.getBoundingClientRect();
        const x = e.clientX - rect.left;
        const y = e.clientY - rect.top;
        const center = DISC_SIZE / 2;
        const radius = center - 1;
        const dx = x - center;
        const dy = y - center;
        const dist = Math.min(Math.sqrt(dx * dx + dy * dy), radius);
        const h = ((Math.atan2(dy, dx) * 180 / Math.PI) + 360) % 360;
        const s = dist / radius;
        setHue(h);
        setSat(s);
        return { h, s };
    }, []);

    const onDiscDown = useCallback((e) => {
        const rect = canvasRef.current.getBoundingClientRect();
        const cx = DISC_SIZE / 2;
        if (Math.sqrt((e.clientX - rect.left - cx) ** 2 + (e.clientY - rect.top - cx) ** 2) > cx) return;
        trackingRef.current = 'disc';
        canvasRef.current.setPointerCapture(e.pointerId);
        onDiscPointer(e);
    }, [onDiscPointer]);

    const onDiscMove = useCallback((e) => {
        if (trackingRef.current !== 'disc') return;
        onDiscPointer(e);
    }, [onDiscPointer]);

    const onDiscUp = useCallback(() => {
        if (trackingRef.current !== 'disc') return;
        trackingRef.current = null;
        commit(hue, sat, brightness);
    }, [commit, hue, sat, brightness]);

    const trackRef = useRef(null);

    const onSliderPointer = useCallback((e) => {
        const rect = trackRef.current.getBoundingClientRect();
        const pct = Math.max(0, Math.min(1, (e.clientX - rect.left) / rect.width));
        const b = Math.max(1, Math.round(pct * 100));
        setBrightness(b);
        return b;
    }, []);

    const onSliderDown = useCallback((e) => {
        trackingRef.current = 'slider';
        trackRef.current.setPointerCapture(e.pointerId);
        onSliderPointer(e);
    }, [onSliderPointer]);

    const onSliderMove = useCallback((e) => {
        if (trackingRef.current !== 'slider') return;
        onSliderPointer(e);
    }, [onSliderPointer]);

    const onSliderUp = useCallback(() => {
        if (trackingRef.current !== 'slider') return;
        trackingRef.current = null;
        commit(hue, sat, brightness);
    }, [commit, hue, sat, brightness]);

    const onKeyDown = useCallback((e) => {
        if (e.key === 'Enter') { e.preventDefault(); commit(hue, sat, brightness); return; }
        if (!e.ctrlKey && !e.metaKey) return;
        const step = e.shiftKey ? 10 : 2;
        if (e.key === 'ArrowLeft') { e.preventDefault(); setHue(h => (h - step + 360) % 360); }
        if (e.key === 'ArrowRight') { e.preventDefault(); setHue(h => (h + step) % 360); }
        if (e.key === 'ArrowUp') { e.preventDefault(); setSat(s => Math.min(1, s + 0.01 * step)); }
        if (e.key === 'ArrowDown') { e.preventDefault(); setSat(s => Math.max(0, s - 0.01 * step)); }
        if (e.key === 'PageUp') { e.preventDefault(); setBrightness(b => Math.min(100, b + step)); }
        if (e.key === 'PageDown') { e.preventDefault(); setBrightness(b => Math.max(1, b - step)); }
    }, [commit, hue, sat, brightness]);

    const hex = hueSatToHex(hue, sat);
    const gradient = `linear-gradient(to right, #0a0a0a, #${hex})`;
    const thumbLeft = `${brightness}%`;
    const gated = ctx.isRuntimeDisabled && field.stream;

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, `field-group field-color${gated ? ' field-gated' : ''}`)}
            onMouseDown=${onSelect} onFocus=${onSelect} onKeyDown=${gated ? undefined : onKeyDown}>
            <label class="field-color-label">${field.label}</label>
            <div class="hue-wheel-container${gated ? ' hue-wheel-disabled' : ''}">
                <canvas ref=${canvasRef} class="hue-disc" width=${DISC_SIZE} height=${DISC_SIZE}
                    onPointerDown=${gated ? undefined : onDiscDown}
                    onPointerMove=${gated ? undefined : onDiscMove}
                    onPointerUp=${gated ? undefined : onDiscUp}
                    onPointerCancel=${gated ? undefined : onDiscUp}
                    style=${gated ? 'pointer-events:none;opacity:0.4' : ''} />
                <div class="hue-slider">
                    <div class="hue-slider-header">
                        <span class="hue-slider-label">Brightness</span>
                        <span class="hue-slider-value">${brightness}%</span>
                    </div>
                    <div ref=${trackRef} class="hue-slider-track" style="background:${gradient}"
                        onPointerDown=${gated ? undefined : onSliderDown}
                        onPointerMove=${gated ? undefined : onSliderMove}
                        onPointerUp=${gated ? undefined : onSliderUp}
                        onPointerCancel=${gated ? undefined : onSliderUp}>
                        <div class="hue-slider-thumb" style="left:${thumbLeft};${gated ? 'opacity:0.4' : ''}" />
                    </div>
                </div>
            </div>
        </div>
    `;
}

function hueComponents(h) {
    const x = 1 - Math.abs(((h / 60) % 2) - 1);
    if (h < 60)  return [1, x, 0];
    if (h < 120) return [x, 1, 0];
    if (h < 180) return [0, 1, x];
    if (h < 240) return [0, x, 1];
    if (h < 300) return [x, 0, 1];
    return [1, 0, x];
}

function hueSatToHex(h, s) {
    const [hr, hg, hb] = hueComponents(h);
    const r = Math.round((1 - s + s * hr) * 255);
    const g = Math.round((1 - s + s * hg) * 255);
    const b = Math.round((1 - s + s * hb) * 255);
    return [r, g, b].map(c => c.toString(16).padStart(2, '0')).join('');
}

function hexToHueSat(hex) {
    const clean = hex.replace(/^#/, '').slice(0, 6).padEnd(6, '0');
    const r = parseInt(clean.substring(0, 2), 16) / 255;
    const g = parseInt(clean.substring(2, 4), 16) / 255;
    const b = parseInt(clean.substring(4, 6), 16) / 255;
    const max = Math.max(r, g, b);
    const min = Math.min(r, g, b);
    const delta = max - min;
    let h = 0;
    if (delta > 0) {
        if (max === r) h = 60 * (((g - b) / delta + 6) % 6);
        else if (max === g) h = 60 * ((b - r) / delta + 2);
        else h = 60 * ((r - g) / delta + 4);
    }
    const s = max === 0 ? 0 : delta / max;
    return { hue: h, saturation: s };
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

function drawDisc(canvas, offscreen, hue, sat) {
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    const center = DISC_SIZE / 2;
    const radius = center - 1;
    ctx.clearRect(0, 0, DISC_SIZE, DISC_SIZE);
    ctx.drawImage(offscreen, 0, 0);
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

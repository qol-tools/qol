import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';
import { getWorldSettings, setWorldSetting, subscribeWorldSettings } from '../../lib/world-settings.js';
import { IconCog } from '../../assets/icon-cog.js';

const CENTER_W_FRAC = 0.34;
const NEIGHBOR_W_FRAC = 0.22;
const PEEK_W_FRAC = 0.06;
const SLOT_GAP = 4;
const SLOT_PAD_Y = 6;
const RADIUS = 3;
const ARROW_FLASH_MS = 350;

export function MinimapContainer({ camera, registry, viewportRef, diveParent }) {
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [settings, setSettings] = useState(getWorldSettings);

    useEffect(() => subscribeWorldSettings(setSettings), []);

    const toggle = useCallback((e) => {
        e.stopPropagation();
        setSettingsOpen(v => !v);
    }, []);

    return html`
        <div class="world-minimap-container">
            ${settingsOpen && html`<${WorldSettingsPanel} settings=${settings} />`}
            <div style="display:flex;align-items:center;gap:6px;">
                <${Minimap} camera=${camera} registry=${registry} viewportRef=${viewportRef} width=${settings.minimapSize} diveParent=${diveParent} />
                <button class="world-minimap-settings-btn" onClick=${toggle} title="World settings">
                    <${IconCog} />
                </button>
            </div>
        </div>
    `;
}

function WorldSettingsPanel({ settings }) {
    const update = (key) => (e) => {
        const val = e.target.type === 'range' ? Number(e.target.value) : e.target.value;
        setWorldSetting(key, val);
    };

    return html`
        <div class="world-settings-panel">
            <label>Pan speed <input type="range" min="4" max="30" value=${settings.panSpeed} onInput=${update('panSpeed')} /></label>
            <label>Transition speed <input type="range" min="40" max="300" value=${settings.transitionSpeed} onInput=${update('transitionSpeed')} /></label>
            <label>Transition style <select value=${settings.transitionStyle} onChange=${update('transitionStyle')}>
                <option value="zoom-fade">Zoom + Fade</option>
                <option value="fade">Fade only</option>
                <option value="instant">Instant</option>
            </select></label>
            <label>Minimap width <input type="range" min="200" max="500" value=${settings.minimapSize} onInput=${update('minimapSize')} /></label>
        </div>
    `;
}

function Minimap({ camera, registry, viewportRef, width, diveParent }) {
    const canvasRef = useRef(null);
    const [, bump] = useState(0);
    const [flash, setFlash] = useState(null);
    const flashTimerRef = useRef(0);

    useEffect(() => {
        let anchorX = camera.x;
        let anchorY = camera.y;
        let resetTimer = 0;

        const unsub = camera.subscribe(() => {
            clearTimeout(resetTimer);

            const dx = camera.x - anchorX;
            const dy = camera.y - anchorY;
            const ax = Math.abs(dx);
            const ay = Math.abs(dy);

            if (ax > 50 || ay > 50) {
                const dirs = new Set();
                if (ax > 50) dirs.add(dx > 0 ? 'right' : 'left');
                if (ay > 50) dirs.add(dy > 0 ? 'down' : 'up');
                setFlash(dirs);
                clearTimeout(flashTimerRef.current);
                flashTimerRef.current = setTimeout(() => setFlash(null), ARROW_FLASH_MS);
                anchorX = camera.x;
                anchorY = camera.y;
            } else {
                resetTimer = setTimeout(() => {
                    anchorX = camera.x;
                    anchorY = camera.y;
                }, 150);
            }

            bump(t => t + 1);
        });

        return () => { unsub(); clearTimeout(resetTimer); };
    }, [camera]);

    useEffect(() => () => clearTimeout(flashTimerRef.current), []);

    const h = Math.round(width * 0.3);

    useEffect(() => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const ctx = canvas.getContext('2d');
        const dpr = window.devicePixelRatio || 1;
        const cw = canvas.clientWidth;
        const ch = canvas.clientHeight;
        if (cw === 0 || ch === 0) return;
        if (canvas.width !== cw * dpr || canvas.height !== ch * dpr) {
            canvas.width = cw * dpr;
            canvas.height = ch * dpr;
        }
        ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

        const currentLayer = camera.layer;
        const vp = viewportRef?.current;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const z = camera.zoom || 1;

        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = diveParent ? allEntries.filter(e => e.parent === diveParent) : allEntries;
        ctx.clearRect(0, 0, cw, ch);
        if (entries.length === 0) return;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);
        const activeIdx = Math.max(0, sorted.findIndex(e => e.id === activeId));

        const slots = buildCenteredSlots(cw, sorted.length, activeIdx);
        drawSlots(ctx, cw, ch, sorted, slots, activeIdx, activeId);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const clickX = e.clientX - rect.left;

        const currentLayer = camera.layer;
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = diveParent ? allEntries.filter(e => e.parent === diveParent) : allEntries;
        if (entries.length === 0) return;

        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = registry.activeViewId(camera.x, camera.y, vpW, vpH, z);
        const activeIdx = Math.max(0, sorted.findIndex(en => en.id === activeId));

        const slots = buildCenteredSlots(canvas.clientWidth, sorted.length, activeIdx);
        const clicked = slots.findIndex(s => clickX >= s.x && clickX < s.x + s.w);
        if (clicked < 0) return;

        const target = sorted[clicked];
        const tx = target.x + target.width / 2 - vpW / (2 * z);
        const ty = target.y + target.height / 2 - vpH / (2 * z);
        camera.panTo(tx, ty);
    };

    return html`
        <div class="world-minimap" style="width:${width}px;height:${h}px" onClick=${onClick}>
            <canvas ref=${canvasRef} style="width:100%;height:100%"></canvas>
            <div class="minimap-arrow minimap-arrow-left ${flash?.has('left') ? 'flash' : ''}"></div>
            <div class="minimap-arrow minimap-arrow-right ${flash?.has('right') ? 'flash' : ''}"></div>
            <div class="minimap-arrow minimap-arrow-up ${flash?.has('up') ? 'flash' : ''}"></div>
            <div class="minimap-arrow minimap-arrow-down ${flash?.has('down') ? 'flash' : ''}"></div>
        </div>
    `;
}

function slotWidth(cw, dist) {
    if (dist === 0) return cw * CENTER_W_FRAC;
    if (dist === 1) return cw * NEIGHBOR_W_FRAC;
    return cw * PEEK_W_FRAC;
}

function buildCenteredSlots(cw, count, activeIdx) {
    if (count === 0) return [];
    const center = cw / 2;
    const activeW = slotWidth(cw, 0);
    const slots = new Array(count);

    slots[activeIdx] = { x: center - activeW / 2, w: activeW };

    let cursor = center + activeW / 2 + SLOT_GAP;
    for (let i = activeIdx + 1; i < count; i++) {
        const w = slotWidth(cw, i - activeIdx);
        slots[i] = { x: cursor, w };
        cursor += w + SLOT_GAP;
    }

    cursor = center - activeW / 2 - SLOT_GAP;
    for (let i = activeIdx - 1; i >= 0; i--) {
        const w = slotWidth(cw, activeIdx - i);
        slots[i] = { x: cursor - w, w };
        cursor -= w + SLOT_GAP;
    }

    return slots;
}

function drawSlots(ctx, cw, ch, sorted, slots, activeIdx, activeId) {
    for (let i = 0; i < sorted.length; i++) {
        const e = sorted[i];
        const s = slots[i];
        if (s.x + s.w < 0 || s.x > cw) continue;
        const active = e.id === activeId;
        const dist = Math.abs(i - activeIdx);

        ctx.fillStyle = active ? 'rgba(255,255,255,0.18)' : dist === 1 ? 'rgba(255,255,255,0.08)' : 'rgba(255,255,255,0.03)';
        roundRect(ctx, s.x, SLOT_PAD_Y, s.w, ch - SLOT_PAD_Y * 2, RADIUS);
        ctx.fill();
        ctx.strokeStyle = active ? 'rgba(255,255,255,0.6)' : dist === 1 ? 'rgba(255,255,255,0.18)' : 'rgba(255,255,255,0.08)';
        ctx.lineWidth = active ? 1.5 : 0.5;
        ctx.stroke();

        if (s.w < 18) continue;
        const label = VIEW_LABELS[e.id] || e.id;
        const fontSize = active ? 10 : dist === 1 ? 9 : 7;
        ctx.fillStyle = active ? 'rgba(255,255,255,0.9)' : dist === 1 ? 'rgba(255,255,255,0.5)' : 'rgba(255,255,255,0.25)';
        ctx.font = `${active ? 'bold ' : ''}${fontSize}px -apple-system, sans-serif`;
        ctx.textAlign = 'center';
        ctx.textBaseline = 'middle';
        ctx.fillText(label, s.x + s.w / 2, ch / 2, s.w - 6);
    }
}

function roundRect(ctx, x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.quadraticCurveTo(x + w, y, x + w, y + r);
    ctx.lineTo(x + w, y + h - r);
    ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
    ctx.lineTo(x + r, y + h);
    ctx.quadraticCurveTo(x, y + h, x, y + h - r);
    ctx.lineTo(x, y + r);
    ctx.quadraticCurveTo(x, y, x + r, y);
    ctx.closePath();
}

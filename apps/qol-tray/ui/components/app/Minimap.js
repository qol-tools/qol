import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { VIEW_LABELS } from './views.js';
import { getWorldSettings, setWorldSetting, subscribeWorldSettings } from '../../lib/world-settings.js';
import { IconCog } from '../../assets/icon-cog.js';
import { useClickOutside } from '../../lib/hooks/useClickOutside.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress, toProgressScale } from '../../utils/progress.js';
import { prettyLabel } from '../../auto-config/heuristics.js';
import { contains } from '../../lib/world-registry.js';

const CENTER_W_FRAC = 0.34;
const NEIGHBOR_W_FRAC = 0.22;
const PEEK_W_FRAC = 0.06;
const SLOT_GAP = 4;
const SLOT_PAD_Y = 6;
const RADIUS = 3;
const ARROW_FLASH_MS = 350;

export function MinimapContainer({ camera, registry, viewportRef, diveParent, activePluginId, diveDepth, navigation, version, updateState, isDevMode, onAction }) {
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [settings, setSettings] = useState(getWorldSettings);
    const cogRef = useRef(null);

    useEffect(() => subscribeWorldSettings(setSettings), []);

    const toggle = useCallback((e) => {
        e.stopPropagation();
        setSettingsOpen(v => !v);
    }, []);
    const close = useCallback(() => setSettingsOpen(false), []);
    useClickOutside(cogRef, settingsOpen, close);

    return html`
        <div class="world-minimap-container">
            ${diveDepth > 0 && html`<span class="world-minimap-depth" style=${`--wedge-hue: ${50 + (diveDepth - 1) * 45}`}>${diveDepth}</span>`}
            <${Minimap} camera=${camera} registry=${registry} viewportRef=${viewportRef} width=${settings.minimapSize} diveParent=${diveParent} activePluginId=${activePluginId} navigation=${navigation} />
        </div>
        <div class="world-cog-anchor" ref=${cogRef}>
            <button class="world-cog-btn ${settingsOpen ? 'is-open' : ''}" onClick=${toggle} title="Settings">
                <${IconCog} size=${28} />
            </button>
            ${settingsOpen && html`<${WorldSettingsPanel} settings=${settings}
                version=${version} updateState=${updateState} isDevMode=${isDevMode} onAction=${onAction} />`}
        </div>
    `;
}

function WorldSettingsPanel({ settings, version, updateState, isDevMode, onAction }) {
    const update = (key) => (e) => {
        const t = e.target;
        const val = t.type === 'range' ? Number(t.value) : t.type === 'checkbox' ? t.checked : t.value;
        setWorldSetting(key, val);
    };

    return html`
        <div class="world-settings-panel">
            <div class="wsp-section">
                <div class="wsp-heading">Navigation</div>
                <label>Pan speed <input type="range" min="4" max="30" value=${settings.panSpeed} onInput=${update('panSpeed')} /></label>
                <label>Minimap size <input type="range" min="200" max="500" value=${settings.minimapSize} onInput=${update('minimapSize')} /></label>
                <label><input type="checkbox" checked=${settings.anchorToPages} onChange=${update('anchorToPages')} /> Anchor view to pages</label>
            </div>
            <div class="wsp-section">
                <div class="wsp-heading">Transitions</div>
                <label>Speed <input type="range" min="40" max="300" value=${settings.transitionSpeed} onInput=${update('transitionSpeed')} /></label>
                <label>Style <select value=${settings.transitionStyle} onChange=${update('transitionStyle')}>
                    <option value="zoom-fade">Zoom + Fade</option>
                    <option value="fade">Fade only</option>
                    <option value="instant">Instant</option>
                </select></label>
            </div>
            ${version && html`<${VersionSection} version=${version} updateState=${updateState} isDevMode=${isDevMode} onAction=${onAction} />`}
        </div>
    `;
}

function VersionSection({ version, updateState, isDevMode, onAction }) {
    const status = updateState?.status || 'idle';
    const tag = isDevMode ? ' DEV' : '';
    const action = versionAction(status, isDevMode);
    const actionLabel = versionActionLabel(status, updateState, isDevMode);
    const busy = status === 'downloading' || status === 'compiling' || status === 'checking' || status === 'done' || status === 'recompile_done';
    const hasUpdate = !isDevMode && status === 'available';
    const progress = versionProgress(status, updateState);
    const detail = versionDetail(status, updateState, isDevMode);

    const actionClick = () => {
        if (!action) return;
        onAction(action);
    };

    return html`
        <div class="wsp-section wsp-version ${progress !== null ? 'progress-track' : ''}">
            ${progress !== null && html`<div class="progress-fill" style=${{ '--progress-scale': toProgressScale(progress) }}></div>`}
            <div class="wsp-version-row">
                <span class="wsp-version-label">v${version}${tag}</span>
                <button class="wsp-version-btn ${hasUpdate ? 'has-update' : ''} ${status === 'error' ? 'is-error' : ''}"
                    onClick=${actionClick} disabled=${busy}>${actionLabel}</button>
            </div>
            ${detail && html`<div class="wsp-version-detail">${detail}</div>`}
        </div>
    `;
}

function versionAction(status, isDevMode) {
    if (status === 'downloading' || status === 'compiling' || status === 'checking' || status === 'done' || status === 'recompile_done') return null;
    if (isDevMode) return 'dev-recompile';
    if (status === 'available') return 'self-update';
    return 'check-update';
}

function versionActionLabel(status, state, isDevMode) {
    if (status === 'available' && !isDevMode) return `Update to v${state?.latest}`;
    if (status === 'downloading') return `${Math.round(clampPercent(state?.percent || 0))}%`;
    if (status === 'compiling') return `${Math.round(clampPercent(state?.percent || 0))}%`;
    if (status === 'checking') return 'Checking...';
    if (status === 'done' || status === 'recompile_done') return 'Restarting...';
    if (status === 'error') return isDevMode ? 'Retry recompile' : 'Retry';
    if (isDevMode) return 'Recompile';
    return 'Check for updates';
}

function versionProgress(status, state) {
    if (status !== 'downloading' && status !== 'compiling') return null;
    return clampPercent(state?.percent || 0);
}

function versionDetail(status, state, isDevMode) {
    if (status === 'downloading') return formatDownloadingProgress(state?.percent || 0);
    if (status === 'compiling') return formatPhaseProgress(state?.phase, state?.percent || 0, 'Recompiling QoL Tray');
    if (status === 'error') return state?.message || (isDevMode ? 'Recompile failed' : 'Update failed');
    if (status === 'recompile_done') return 'Recompile complete';
    if (status === 'done' && isDevMode) return 'Update complete';
    return null;
}

function Minimap({ camera, registry, viewportRef, width, diveParent, activePluginId, navigation }) {
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

    const h = Math.round(width * 0.55);

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

        const confinement = navigation?.getCurrentConfinement?.() || null;
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = confinement
            ? allEntries.filter(e => contains(confinement, e))
            : (diveParent ? allEntries.filter(e => e.parent === diveParent) : allEntries);
        ctx.clearRect(0, 0, cw, ch);
        if (entries.length === 0) return;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = nearestEntryId(sorted, camera, vpW, vpH, z);
        const activeIdx = Math.max(0, sorted.findIndex(e => e.id === activeId));

        const slots = buildCenteredSlots(cw, sorted.length, activeIdx);
        drawSlots(ctx, cw, ch, sorted, slots, activeIdx, activeId, activePluginId);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const clickX = e.clientX - rect.left;

        const currentLayer = camera.layer;
        const confinement = navigation?.getCurrentConfinement?.() || null;
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = confinement
            ? allEntries.filter(e => contains(confinement, e))
            : (diveParent ? allEntries.filter(e => e.parent === diveParent) : allEntries);
        if (entries.length === 0) return;

        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = nearestEntryId(sorted, camera, vpW, vpH, z);
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

function nearestEntryId(entries, camera, vpW, vpH, zoom) {
    if (entries.length === 0) return null;
    if (entries.length === 1) return entries[0].id;
    const z = zoom || 1;
    const cx = camera.x + vpW / (2 * z);
    const cy = camera.y + vpH / (2 * z);
    let bestId = null;
    let bestDist = Infinity;
    for (const e of entries) {
        const ex = e.x + e.width / 2;
        const ey = e.y + e.height / 2;
        const d = Math.hypot(cx - ex, cy - ey);
        if (d < bestDist) { bestId = e.id; bestDist = d; }
    }
    return bestId;
}

function slotLabel(entry, activePluginId) {
    if (entry.label) return entry.label;
    if (entry.id === 'plugins-config' && activePluginId) {
        return prettyLabel(activePluginId.replace(/^plugin-/, ''));
    }
    return VIEW_LABELS[entry.id] || entry.id;
}

function drawSlots(ctx, cw, ch, sorted, slots, activeIdx, activeId, activePluginId) {
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
        const label = slotLabel(e, activePluginId);
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

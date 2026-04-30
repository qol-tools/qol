import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { resolveViewLabel } from '../../app/views.js';
import { getWorldSettings, setWorldSetting, subscribeWorldSettings } from '../../lib/world-settings.js';
import { IconCog } from '../../assets/icon-cog.js';
import { useClickOutside } from '../../lib/hooks/useClickOutside.js';
import { Peripheral } from './Peripheral.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress, toProgressScale } from '../../utils/progress.js';
import { computeMinimapLinearLayout, computeMinimapLinearRect } from '../../lib/minimap-geometry.js';
import { visibleMinimapEntries } from '../../lib/minimap-filter.js';
import { drawMinimap, drawViewportRect } from '../../lib/minimap-draw.js';
import { cameraTargetFor } from '../../lib/world-geometry.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { resolveViewport } from '../../lib/viewport-resolve.js';

const ARROW_FLASH_MS = 350;

export function MinimapContainer({ camera, registry, viewportRef, diveParent, diveDepth, navigation, version, updateState, isDevMode, onAction, worktrees, defaultWorktree, setDefaultWorktree, repoBranch }) {
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
        <${Peripheral} camera=${camera} navigation=${navigation} edge="br"
            className="world-minimap-container">
            ${diveDepth > 0 && html`<span class="world-minimap-depth" style=${`--wedge-hue: ${50 + (diveDepth - 1) * 45}`}>${diveDepth}</span>`}
            <${Minimap} camera=${camera} registry=${registry} viewportRef=${viewportRef} width=${settings.minimapSize} diveParent=${diveParent} navigation=${navigation} />
        <//>
        <${Peripheral} camera=${camera} navigation=${navigation} edge="bl"
            alwaysVisible=${settingsOpen} className="world-cog-anchor" elementRef=${cogRef}>
            <button class="world-cog-btn ${settingsOpen ? 'is-open' : ''}" onClick=${toggle} title="Settings">
                <${IconCog} size=${28} />
            </button>
            ${settingsOpen && html`<${WorldSettingsPanel} settings=${settings}
                version=${version} updateState=${updateState} isDevMode=${isDevMode} onAction=${onAction}
                worktrees=${worktrees} defaultWorktree=${defaultWorktree} setDefaultWorktree=${setDefaultWorktree}
                repoBranch=${repoBranch} />`}
        <//>
    `;
}

const TRANSITION_STYLE_OPTIONS = ['zoom-fade', 'fade', 'instant'];
const TRANSITION_STYLE_LABELS = { 'zoom-fade': 'Zoom + Fade', fade: 'Fade only', instant: 'Instant' };

export const MINIMAP_ZOOM_MIN = 1;
export const MINIMAP_ZOOM_MAX = 20;
export const MINIMAP_MIN_ACTIVE_SLOT_PX = 50;

function WorldSettingsPanel({ settings, version, updateState, isDevMode, onAction, worktrees, defaultWorktree, setDefaultWorktree, repoBranch }) {
    const updateRange = (key) => (e) => setWorldSetting(key, Number(e.target.value));
    const updateToggle = (key) => (value) => setWorldSetting(key, value);
    const updateSelect = (key) => (value) => setWorldSetting(key, value);

    const minimapZoom = Number(settings.minimapZoomFactor ?? 4);
    const minimapZoomLabel = minimapZoom >= MINIMAP_ZOOM_MAX ? 'all' : `${minimapZoom.toFixed(1)}×`;

    return html`
        <div class="world-settings-panel">
            <div class="wsp-section">
                <div class="wsp-heading">Navigation</div>
                <div class="wsp-grid">
                    ${rangeRow({ label: 'Pan speed', key: 'panSpeed', min: 4, max: 30, value: settings.panSpeed, onInput: updateRange('panSpeed') })}
                    ${rangeRow({ label: 'Minimap size', key: 'minimapSize', min: 200, max: 500, value: settings.minimapSize, onInput: updateRange('minimapSize') })}
                    ${rangeRow({ label: 'Minimap zoom', key: 'minimapZoomFactor', min: MINIMAP_ZOOM_MIN, max: MINIMAP_ZOOM_MAX, step: 0.5, value: minimapZoom, onInput: updateRange('minimapZoomFactor'), display: minimapZoomLabel })}
                    ${rangeRow({ label: 'Default zoom', key: 'defaultZoom', min: 0.5, max: 2, step: 0.05, value: settings.defaultZoom, onInput: updateRange('defaultZoom'), display: `${Number(settings.defaultZoom).toFixed(2)}×` })}
                    ${rangeRow({ label: 'Ghost threshold', key: 'ghostThreshold', min: 0.2, max: 1, step: 0.05, value: settings.ghostThreshold, onInput: updateRange('ghostThreshold'), display: `${Number(settings.ghostThreshold).toFixed(2)}×` })}
                </div>
                <div class="wsp-toggles">
                    <${ToggleSwitch} checked=${settings.anchorToPages} onChange=${updateToggle('anchorToPages')} label="Anchor view to pages" />
                    <${ToggleSwitch} checked=${settings.resetZoomOnNav} onChange=${updateToggle('resetZoomOnNav')} label="Reset zoom on keyboard nav" />
                    <${ToggleSwitch} checked=${settings.uiScaleOnZoomOut} onChange=${updateToggle('uiScaleOnZoomOut')} label="Scale pages up when zoomed out" />
                </div>
            </div>
            <div class="wsp-section">
                <div class="wsp-heading">Transitions</div>
                <div class="wsp-grid">
                    ${rangeRow({ label: 'Speed', key: 'transitionSpeed', min: 40, max: 300, value: settings.transitionSpeed, onInput: updateRange('transitionSpeed') })}
                    <span class="wsp-label">Style</span>
                    <div class="wsp-control">
                        <${CustomSelect} value=${settings.transitionStyle} options=${TRANSITION_STYLE_OPTIONS}
                            labels=${TRANSITION_STYLE_LABELS} onChange=${updateSelect('transitionStyle')} compact=${true} />
                    </div>
                </div>
            </div>
            ${isDevMode && worktrees && worktrees.length > 0 && html`
                <${WorktreeSection} worktrees=${worktrees} defaultWorktree=${defaultWorktree}
                    setDefaultWorktree=${setDefaultWorktree} repoBranch=${repoBranch} />
            `}
            ${version && html`<${VersionSection} version=${version} updateState=${updateState} isDevMode=${isDevMode} onAction=${onAction} />`}
        </div>
    `;
}

function rangeRow({ label, key, min, max, step, value, onInput, display }) {
    return html`
        <span class="wsp-label">${label}</span>
        <input class="wsp-range" type="range" min=${min} max=${max} step=${step ?? 'any'}
            value=${value} onInput=${onInput} aria-label=${label} data-setting=${key} />
        <span class="wsp-value">${display ?? ''}</span>
    `;
}

function WorktreeSection({ worktrees, defaultWorktree, setDefaultWorktree, repoBranch }) {
    const baseLabel = repoBranch || 'main';
    const options = ['', ...worktrees.map(w => w.path)];
    const labels = { '': `${baseLabel} (base)` };
    for (const w of worktrees) labels[w.path] = w.branch;
    const onChange = useCallback((value) => setDefaultWorktree(value || null), [setDefaultWorktree]);
    return html`
        <div class="wsp-section">
            <div class="wsp-heading">
                <span>Worktree</span>
                ${repoBranch && html`<span class="wsp-pill" title="Base repo HEAD">${repoBranch}</span>`}
            </div>
            <div class="wsp-grid">
                <span class="wsp-label">Branch</span>
                <div class="wsp-control">
                    <${CustomSelect} value=${defaultWorktree || ''} options=${options}
                        labels=${labels} onChange=${onChange} compact=${true} />
                </div>
            </div>
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

function Minimap({ camera, registry, viewportRef, width, diveParent, navigation }) {
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
        const vp = resolveViewport(viewportRef);
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;
        const z = camera.zoom || 1;

        const confinedPages = navigation?.getConfinedPages?.() || [];
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = visibleMinimapEntries({ allEntries, confinedPages, diveParent });
        ctx.clearRect(0, 0, cw, ch);
        if (entries.length === 0) return;

        const sortedAll = [...entries].sort((a, b) => a.x - b.x);
        const activeId = nearestEntryId(sortedAll, camera, vpW, vpH, z);
        const settings = getWorldSettings();
        const bounds = resolveMinimapWorldBounds({
            sortedAll,
            activeId,
            viewportWidthPx: vpW,
            cameraZoom: z,
            factor: settings.minimapZoomFactor,
            minimapWidth: cw,
        });
        if (!bounds) return;

        const layout = computeMinimapLinearLayout({
            entries: sortedAll,
            worldStart: bounds.worldStart,
            worldEnd: bounds.worldEnd,
            minimapWidth: cw,
            canvasHeight: ch,
        });
        if (!layout) return;

        const viewportRange = vpW > 0 && z > 0 ? vpW / z : 0;
        const rect = computeMinimapLinearRect({
            cameraX: camera.x,
            viewportRange,
            worldStart: bounds.worldStart,
            worldEnd: bounds.worldEnd,
            minimapWidth: cw,
            rowY: layout.rowY,
            rowHeight: layout.rowHeight,
        });
        drawMinimap(ctx, cw, ch, sortedAll, layout.slots, activeId, slotLabel, rect);
        drawViewportRect(ctx, cw, ch, rect);
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const minimapWidth = canvas.clientWidth;
        const canvasHeight = canvas.clientHeight;
        // Clamp to the canvas so the final pixel still lands in the last slot
        // (strict `clickX < slot.x + slot.w` would miss the right-edge column).
        const clickX = Math.min(Math.max(e.clientX - rect.left, 0), minimapWidth - 1e-6);
        const clickY = Math.min(Math.max(e.clientY - rect.top, 0), canvasHeight - 1e-6);

        const currentLayer = camera.layer;
        const confinedPages = navigation?.getConfinedPages?.() || [];
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = visibleMinimapEntries({ allEntries, confinedPages, diveParent });
        if (entries.length === 0) return;

        const vp = resolveViewport(viewportRef);
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;

        const sortedAll = [...entries].sort((a, b) => a.x - b.x);
        const settings = getWorldSettings();
        const activeId = nearestEntryId(sortedAll, camera, vpW, vpH, z);
        const bounds = resolveMinimapWorldBounds({
            sortedAll,
            activeId,
            viewportWidthPx: vpW,
            cameraZoom: z,
            factor: settings.minimapZoomFactor,
            minimapWidth,
        });
        if (!bounds) return;
        const layout = computeMinimapLinearLayout({
            entries: sortedAll,
            worldStart: bounds.worldStart,
            worldEnd: bounds.worldEnd,
            minimapWidth,
            canvasHeight,
        });
        if (!layout) return;
        // Clicks must land inside the centred slot row's y-range as well as the
        // correct x-column, so click-margin above/below the row is ignored.
        const clicked = layout.slots.findIndex(s =>
            clickX >= s.x && clickX < s.x + s.w &&
            clickY >= s.y && clickY < s.y + s.h);
        if (clicked < 0) return;

        const target = sortedAll[clicked];
        const cam = cameraTargetFor(target, vpW, vpH, z);
        camera.panTo(cam.x, cam.y);
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

function slotLabel(entry) {
    return resolveViewLabel(entry).text;
}

export function resolveMinimapWorldBounds({ sortedAll, activeId, viewportWidthPx, cameraZoom, factor, minimapWidth }) {
    if (!Array.isArray(sortedAll) || sortedAll.length === 0) return null;
    const f = Number(factor);
    const viewportRange = viewportWidthPx > 0 && cameraZoom > 0
        ? viewportWidthPx / cameraZoom
        : 0;
    const active = activeId ? sortedAll.find(e => e.id === activeId) : null;

    const showAll = !Number.isFinite(f) || f >= MINIMAP_ZOOM_MAX
        || !(viewportRange > 0) || !active;
    if (showAll) {
        let minX = Infinity, maxX = -Infinity;
        for (const e of sortedAll) {
            if (e.x < minX) minX = e.x;
            const ex = e.x + e.width;
            if (ex > maxX) maxX = ex;
        }
        if (!(maxX > minX)) return null;
        return { worldStart: minX, worldEnd: maxX };
    }

    let range = viewportRange * Math.max(MINIMAP_ZOOM_MIN, f);
    if (minimapWidth > 0 && active.width > 0) {
        const maxRange = (active.width * minimapWidth) / MINIMAP_MIN_ACTIVE_SLOT_PX;
        if (range > maxRange) range = maxRange;
    }
    const center = active.x + active.width / 2;
    return { worldStart: center - range / 2, worldEnd: center + range / 2 };
}


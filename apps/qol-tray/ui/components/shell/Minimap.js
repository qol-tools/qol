import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { resolveViewLabel } from '../../app/views.js';
import { getWorldSettings, setWorldSetting, subscribeWorldSettings } from '../../lib/world-settings.js';
import { IconCog } from '../../assets/icon-cog.js';
import { useClickOutside } from '../../lib/hooks/useClickOutside.js';
import { Peripheral } from './Peripheral.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress, toProgressScale } from '../../utils/progress.js';
import { computeMinimapSlots, computeMinimapViewportRect } from '../../lib/minimap-geometry.js';
import { visibleMinimapEntries } from '../../lib/minimap-filter.js';
import { computeLayerPulse, drawMinimap, drawViewportRect, LAYER_PULSE_MS } from '../../lib/minimap-draw.js';
import {
    cameraTargetFor,
    computeBaseScale,
    computeSlotScale,
    inflatedEntryRange,
} from '../../lib/world-geometry.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';

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

function WorldSettingsPanel({ settings, version, updateState, isDevMode, onAction, worktrees, defaultWorktree, setDefaultWorktree, repoBranch }) {
    const updateRange = (key) => (e) => setWorldSetting(key, Number(e.target.value));
    const updateToggle = (key) => (value) => setWorldSetting(key, value);
    const updateSelect = (key) => (value) => setWorldSetting(key, value);

    return html`
        <div class="world-settings-panel">
            <div class="wsp-section">
                <div class="wsp-heading">Navigation</div>
                <label>Pan speed <input type="range" min="4" max="30" value=${settings.panSpeed} onInput=${updateRange('panSpeed')} /></label>
                <label>Minimap size <input type="range" min="200" max="500" value=${settings.minimapSize} onInput=${updateRange('minimapSize')} /></label>
                <label>Default zoom <input type="range" min="0.5" max="2" step="0.05" value=${settings.defaultZoom} onInput=${updateRange('defaultZoom')} /> <span class="wsp-value">${Number(settings.defaultZoom).toFixed(2)}×</span></label>
                <label>Ghost threshold <input type="range" min="0.2" max="1" step="0.05" value=${settings.ghostThreshold} onInput=${updateRange('ghostThreshold')} /> <span class="wsp-value">${Number(settings.ghostThreshold).toFixed(2)}×</span></label>
                <${ToggleSwitch} checked=${settings.anchorToPages} onChange=${updateToggle('anchorToPages')} label="Anchor view to pages" />
                <${ToggleSwitch} checked=${settings.resetZoomOnNav} onChange=${updateToggle('resetZoomOnNav')} label="Reset zoom on keyboard nav" />
                <${ToggleSwitch} checked=${settings.uiScaleOnZoomOut} onChange=${updateToggle('uiScaleOnZoomOut')} label="Scale pages up when zoomed out" />
            </div>
            <div class="wsp-section">
                <div class="wsp-heading">Transitions</div>
                <label>Speed <input type="range" min="40" max="300" value=${settings.transitionSpeed} onInput=${updateRange('transitionSpeed')} /></label>
                <label><span>Style</span>
                    <${CustomSelect} value=${settings.transitionStyle} options=${TRANSITION_STYLE_OPTIONS}
                        labels=${TRANSITION_STYLE_LABELS} onChange=${updateSelect('transitionStyle')} compact=${true} />
                </label>
            </div>
            ${isDevMode && worktrees && worktrees.length > 0 && html`
                <${WorktreeSection} worktrees=${worktrees} defaultWorktree=${defaultWorktree}
                    setDefaultWorktree=${setDefaultWorktree} repoBranch=${repoBranch} />
            `}
            ${version && html`<${VersionSection} version=${version} updateState=${updateState} isDevMode=${isDevMode} onAction=${onAction} />`}
        </div>
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
            <label><span>Branch</span>
                <${CustomSelect} value=${defaultWorktree || ''} options=${options}
                    labels=${labels} onChange=${onChange} compact=${true} />
            </label>
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
    const pulseStartRef = useRef(null);
    const pulseRafRef = useRef(0);

    useEffect(() => {
        let anchorX = camera.x;
        let anchorY = camera.y;
        let lastLayer = camera.layer;
        let resetTimer = 0;

        const drivePulseFrames = () => {
            if (pulseStartRef.current == null) return;
            bump(t => t + 1);
            const elapsed = performance.now() - pulseStartRef.current;
            if (elapsed >= LAYER_PULSE_MS) {
                pulseStartRef.current = null;
                pulseRafRef.current = 0;
                return;
            }
            pulseRafRef.current = requestAnimationFrame(drivePulseFrames);
        };

        const unsub = camera.subscribe(() => {
            clearTimeout(resetTimer);

            // Layer transition (dive or ascend): the rect's geometric position
            // can land on the same slot pre/post (Plugins → plugin → Plugins),
            // so a pulse provides the visible "you crossed a layer" cue that
            // pure geometry can't here.
            if (camera.layer !== lastLayer) {
                lastLayer = camera.layer;
                pulseStartRef.current = performance.now();
                if (!pulseRafRef.current) {
                    pulseRafRef.current = requestAnimationFrame(drivePulseFrames);
                }
            }

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

        return () => {
            unsub();
            clearTimeout(resetTimer);
            if (pulseRafRef.current) cancelAnimationFrame(pulseRafRef.current);
            pulseRafRef.current = 0;
            pulseStartRef.current = null;
        };
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

        const confinedPages = navigation?.getConfinedPages?.() || [];
        const allEntries = registry.getEntriesForLayer(currentLayer);
        const entries = visibleMinimapEntries({ allEntries, confinedPages, diveParent });
        ctx.clearRect(0, 0, cw, ch);
        if (entries.length === 0) return;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const activeId = nearestEntryId(sorted, camera, vpW, vpH, z);

        const slots = computeMinimapSlots({ sortedEntries: sorted, minimapWidth: cw, canvasHeight: ch });
        const labelFor = (entry) => slotLabel(entry);
        // When uiScaleOnZoomOut is on and zoom < ghostThreshold, slots are
        // CSS-scaled around their centre — see App.js's applySlotScales. The
        // rect needs the same per-entry scale to know which slots the user
        // *visually* sees, otherwise it under-represents the camera's framing.
        const settings = getWorldSettings();
        const baseScale = settings.uiScaleOnZoomOut
            ? computeBaseScale(z, settings.ghostThreshold)
            : 1;
        const inflatedRanges = baseScale > 1
            ? sorted.map(entry => inflatedEntryRange(entry, computeSlotScale({
                entry,
                cameraX: camera.x,
                cameraY: camera.y,
                viewportW: vpW,
                viewportH: vpH,
                zoom: z,
                baseScale,
            })))
            : null;
        const rect = computeMinimapViewportRect({
            sortedEntries: sorted,
            cameraX: camera.x,
            cameraZoom: z,
            viewportWidthPx: vpW,
            minimapWidth: cw,
            canvasHeight: ch,
            inflatedRanges,
        });
        const pulse = computeLayerPulse(performance.now(), pulseStartRef.current);
        drawMinimap(ctx, cw, ch, sorted, slots, activeId, labelFor, rect);
        drawViewportRect(ctx, cw, ch, rect, pulse);
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

        const vp = viewportRef?.current;
        const z = camera.zoom || 1;
        const vpW = vp ? vp.clientWidth : 0;
        const vpH = vp ? vp.clientHeight : 0;

        const sorted = [...entries].sort((a, b) => a.x - b.x);
        const slots = computeMinimapSlots({ sortedEntries: sorted, minimapWidth, canvasHeight });
        // Clicks must land inside the centred slot row's y-range as well as the
        // correct x-column, so click-margin above/below the row is ignored.
        const clicked = slots.findIndex(s =>
            clickX >= s.x && clickX < s.x + s.w &&
            clickY >= s.y && clickY < s.y + s.h);
        if (clicked < 0) return;

        const target = sorted[clicked];
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


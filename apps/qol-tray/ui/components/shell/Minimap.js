import { html } from '../../lib/html.js';
import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'preact/hooks';
import { resolveViewLabel } from '../../app/views.js';
import { ALWAYS_ID } from '../../palette/registry.js';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { getWorldSettings, setWorldSetting, subscribeWorldSettings } from '../../lib/world-settings.js';
import { ACCENT_PRESETS } from '../../lib/accent-presets.js';
import { IconCog } from '../../assets/icon-cog.js';
import { useClickOutside } from '../../lib/hooks/useClickOutside.js';
import { Peripheral } from './Peripheral.js';
import { clampPercent, formatDownloadingProgress, formatPhaseProgress, toProgressScale } from '../../utils/progress.js';
import { computeMinimapFocalLayout, computeMinimapFocalRect } from '../../lib/minimap-geometry.js';
import { visibleMinimapEntries } from '../../lib/minimap-filter.js';
import { drawMinimap, drawViewportRect } from '../../lib/minimap-draw.js';
import { cameraTargetFor } from '../../lib/world-geometry.js';
import { ToggleSwitch } from '../../lib/components/ToggleSwitch.js';
import { CustomSelect } from '../../lib/components/CustomSelect.js';
import { Surface } from '../../lib/components/Surface.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';
import { resolveViewport } from '../../lib/viewport-resolve.js';
import { createDebug } from '../../lib/debug.js';

const log = createDebug('qol:minimap');

const ARROW_FLASH_MS = 350;

export function MinimapContainer({ camera, registry, viewportRef, diveParent, diveDepth, navigation, version, updateState, isDevMode, onAction, branches, defaultBranch, setDefaultBranch, repoBranch }) {
    const [settingsOpen, setSettingsOpen] = useState(false);
    const [settings, setSettings] = useState(getWorldSettings);
    const cogRef = useRef(null);
    const panelRef = useRef(null);

    useEffect(() => subscribeWorldSettings(setSettings), []);

    const openSettings = useCallback(() => {
        setSettingsOpen(true);
        focusSettingsPanel(panelRef);
    }, []);
    useLayoutEffect(() => {
        if (!settingsOpen) return;
        focusSettingsPanel(panelRef);
    }, [settingsOpen]);
    const commands = useMemo(() => [
        { id: 'world:settings', label: 'Settings', run: openSettings },
    ], [openSettings]);
    useRegisterCommands(ALWAYS_ID, commands);

    const toggle = useCallback((e) => {
        e.stopPropagation();
        if (settingsOpen) {
            setSettingsOpen(false);
            return;
        }
        openSettings();
    }, [openSettings, settingsOpen]);
    const close = useCallback(() => setSettingsOpen(false), []);
    const onSettingsKeyDown = useCallback((e) => {
        if (e.key !== 'Escape') return;
        e.preventDefault();
        e.stopPropagation();
        setSettingsOpen(false);
    }, []);
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
                branches=${branches} defaultBranch=${defaultBranch} setDefaultBranch=${setDefaultBranch}
                repoBranch=${repoBranch} containerRef=${panelRef} onKeyDown=${onSettingsKeyDown} />`}
        <//>
    `;
}

function focusSettingsPanel(panelRef) {
    const first = panelRef.current?.querySelector?.('[data-selected-surface]');
    if (first instanceof HTMLElement) first.focus({ preventScroll: true });
}

const TRANSITION_STYLE_OPTIONS = ['zoom-fade', 'fade', 'instant'];
const TRANSITION_STYLE_LABELS = { 'zoom-fade': 'Zoom + Fade', fade: 'Fade only', instant: 'Instant' };

export const MINIMAP_NEIGHBOURS_MIN = 1;
export const MINIMAP_NEIGHBOURS_MAX = 12;

function WorldSettingsPanel({ settings, version, updateState, isDevMode, onAction, branches, defaultBranch, setDefaultBranch, repoBranch, containerRef, onKeyDown }) {
    const updateRange = (key) => (e) => setWorldSetting(key, Number(e.target.value));
    const updateToggle = (key) => (value) => setWorldSetting(key, value);
    const updateSelect = (key) => (value) => setWorldSetting(key, value);

    const minimapZoom = Number(settings.minimapZoomFactor ?? 4);
    const minimapZoomLabel = minimapZoom >= MINIMAP_NEIGHBOURS_MAX ? 'all' : `±${minimapZoom | 0}`;

    return html`
        <${SurfaceContainer} className="world-settings-panel" containerRef=${containerRef} onKeyDown=${onKeyDown}
            data-keyboard-isolated="" data-surface-depth-base="1">
            <div class="wsp-section">
                <div class="wsp-heading">Navigation</div>
                <div class="wsp-grid">
                    <${RangeRow} label="Pan speed" settingKey="panSpeed" min=${4} max=${30} value=${settings.panSpeed} onInput=${updateRange('panSpeed')} selected=${true} />
                    <${RangeRow} label="Minimap size" settingKey="minimapSize" min=${200} max=${500} value=${settings.minimapSize} onInput=${updateRange('minimapSize')} />
                    <${RangeRow} label="Minimap zoom" settingKey="minimapZoomFactor" min=${MINIMAP_NEIGHBOURS_MIN} max=${MINIMAP_NEIGHBOURS_MAX} step=${1} value=${minimapZoom} onInput=${updateRange('minimapZoomFactor')} display=${minimapZoomLabel} />
                    <${RangeRow} label="Default zoom" settingKey="defaultZoom" min=${0.5} max=${2} step=${0.05} value=${settings.defaultZoom} onInput=${updateRange('defaultZoom')} display=${`${Number(settings.defaultZoom).toFixed(2)}×`} />
                    <${RangeRow} label="Ghost threshold" settingKey="ghostThreshold" min=${0.2} max=${1} step=${0.05} value=${settings.ghostThreshold} onInput=${updateRange('ghostThreshold')} display=${`${Number(settings.ghostThreshold).toFixed(2)}×`} />
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
                    <${RangeRow} label="Speed" settingKey="transitionSpeed" min=${40} max=${300} value=${settings.transitionSpeed} onInput=${updateRange('transitionSpeed')} />
                    <${SelectRow} label="Style">
                        <${CustomSelect} value=${settings.transitionStyle} options=${TRANSITION_STYLE_OPTIONS}
                            labels=${TRANSITION_STYLE_LABELS} onChange=${updateSelect('transitionStyle')} compact=${true} />
                    <//>
                </div>
            </div>
            <div class="wsp-section">
                <div class="wsp-heading">Appearance</div>
                <${AccentRow} value=${settings.accent} isDevMode=${isDevMode}
                    onPick=${(key) => setWorldSetting('accent', key)} />
            </div>
            ${isDevMode && branches && branches.length > 0 && html`
                <${WorktreeSection} branches=${branches} defaultBranch=${defaultBranch}
                    setDefaultBranch=${setDefaultBranch} repoBranch=${repoBranch} />
            `}
            ${version && html`<${VersionSection} version=${version} updateState=${updateState} isDevMode=${isDevMode}
                onAction=${onAction} />`}
        <//>
    `;
}

function AccentRow({ value, isDevMode, onPick }) {
    const autoLabel = `Auto (${isDevMode ? 'dev: green' : 'amber'})`;
    return html`
        <div class="wsp-accent">
            <span class="wsp-label">Accent</span>
            <div class="wsp-swatches">
                <${Surface} as="button" className=${`wsp-swatch wsp-swatch-auto${value ? '' : ' is-active'}`}
                    title=${autoLabel} onActivate=${() => onPick(null)}>A<//>
                ${Object.entries(ACCENT_PRESETS).map(([key, preset]) => html`
                    <${Surface} as="button" key=${key}
                        className=${`wsp-swatch${value === key ? ' is-active' : ''}`}
                        style=${`--sw: rgb(${preset.rgb})`} title=${preset.label}
                        onActivate=${() => onPick(key)} />
                `)}
            </div>
        </div>`;
}

function RangeRow({ label, settingKey, min, max, step, value, onInput, display, selected }) {
    const inputRef = useRef(null);
    const focusInput = useCallback(() => inputRef.current?.focus({ preventScroll: true }), []);
    const onInputKeyDown = useCallback((e) => {
        if (e.key !== 'Enter' && e.key !== 'Escape') return;
        e.preventDefault();
        e.stopPropagation();
        e.currentTarget.closest('[data-selected-surface]')?.focus({ preventScroll: true });
    }, []);
    return html`
        <${Surface} className="wsp-range-row" onActivate=${focusInput} selected=${selected}>
            <span class="wsp-label">${label}</span>
            <input ref=${inputRef} class="wsp-range" type="range" min=${min} max=${max} step=${step ?? 'any'}
                value=${value} onInput=${onInput} onKeyDown=${onInputKeyDown} aria-label=${label} data-setting=${settingKey} />
            <span class="wsp-value">${display ?? ''}</span>
        <//>
    `;
}

function SelectRow({ label, children }) {
    return html`
        <div class="wsp-select-row">
            <span class="wsp-label">${label}</span>
            <div class="wsp-control">${children}</div>
        </div>
    `;
}

function WorktreeSection({ branches, defaultBranch, setDefaultBranch, repoBranch }) {
    const head = repoBranch || 'main';
    const options = useMemo(() => {
        const seen = new Set();
        const out = [];
        for (const branch of [head, ...branches, defaultBranch]) {
            if (!branch || seen.has(branch)) continue;
            seen.add(branch);
            out.push(branch);
        }
        return out;
    }, [head, branches, defaultBranch]);
    const labels = useMemo(() => {
        const acc = { [head]: `${head} (current)` };
        for (const branch of options) if (branch && branch !== head) acc[branch] = branch;
        return acc;
    }, [head, options]);
    const value = defaultBranch || head;
    const onChange = useCallback(
        (next) => setDefaultBranch(next === head ? null : next),
        [setDefaultBranch, head]
    );
    return html`
        <div class="wsp-section">
            <div class="wsp-heading">
                <span>Worktree</span>
            </div>
            <div class="wsp-grid">
                <${SelectRow} label="Branch">
                    <${CustomSelect} value=${value} options=${options}
                        labels=${labels} onChange=${onChange} compact=${true} />
                <//>
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
                <${Surface} as="button" className=${`wsp-version-btn ${hasUpdate ? 'has-update' : ''} ${status === 'error' ? 'is-error' : ''}`}
                    onActivate=${actionClick} disabled=${busy}>${actionLabel}<//>
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
        ctx.clearRect(0, 0, cw, ch);

        const view = computeMinimapView({
            camera, registry, viewportRef, navigation, diveParent,
            minimapWidth: cw, canvasHeight: ch,
        });
        if (!view) return;

        const { sortedAll, layout, activeIdInWorld, viewportRange, currentLayer, vpW, activePosF, pageWidth, viewportPagesSpan, effectiveR } = view;

        const rawRect = computeMinimapFocalRect({
            entries: sortedAll,
            slots: layout.slots,
            cameraX: camera.x,
            viewportRange,
        });
        const rect = rawRect
            ? { x: rawRect.x, y: layout.rowY, width: rawRect.width, height: layout.rowHeight }
            : { x: 0, y: layout.rowY, width: 0, height: layout.rowHeight };
        drawMinimap(ctx, cw, ch, sortedAll, layout.slots, activeIdInWorld, slotLabel, rect);
        drawViewportRect(ctx, cw, ch, rect);

        log.verbose(
            'render',
            'active=', activeIdInWorld,
            'posF=', activePosF.toFixed(3),
            'N=', sortedAll.length,
            'cw=', cw,
            'vpW=', vpW,
            'z=', (camera.zoom || 1).toFixed(3),
            'vR=', viewportRange.toFixed(0),
            'pageW=', pageWidth,
            'span=', viewportPagesSpan.toFixed(3),
            'R=', effectiveR.toFixed(3),
            'activeW=', layout.slots[Math.round(activePosF)]?.w?.toFixed(1),
            'rectW=', rect.width.toFixed(1),
            'cam.layer=', currentLayer,
        );
    });

    const onClick = (e) => {
        const canvas = canvasRef.current;
        if (!canvas) return;
        const bounds = canvas.getBoundingClientRect();
        const minimapWidth = canvas.clientWidth;
        const canvasHeight = canvas.clientHeight;
        const clickX = Math.min(Math.max(e.clientX - bounds.left, 0), minimapWidth - 1e-6);
        const clickY = Math.min(Math.max(e.clientY - bounds.top, 0), canvasHeight - 1e-6);

        const view = computeMinimapView({
            camera, registry, viewportRef, navigation, diveParent,
            minimapWidth, canvasHeight,
        });
        if (!view) return;

        const { sortedAll, layout, vpW, vpH } = view;
        const clicked = layout.slots.findIndex(s =>
            clickX >= s.x && clickX < s.x + s.w &&
            clickY >= s.y && clickY < s.y + s.h);
        if (clicked < 0) return;

        const target = sortedAll[clicked];
        const cam = cameraTargetFor(target, vpW, vpH, camera.zoom || 1);
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

function slotLabel(entry) {
    return resolveViewLabel(entry).text;
}

function computeMinimapView({ camera, registry, viewportRef, navigation, diveParent, minimapWidth, canvasHeight }) {
    const currentLayer = camera.layer;
    const vp = resolveViewport(viewportRef);
    const vpW = vp ? vp.clientWidth : 0;
    const vpH = vp ? vp.clientHeight : 0;
    const z = camera.zoom || 1;

    const confinedPages = navigation?.getConfinedPages?.() || [];
    const allEntries = registry.getEntriesForLayer(currentLayer);
    const entries = visibleMinimapEntries({ allEntries, confinedPages, diveParent });
    if (entries.length === 0) return null;

    const sortedAll = [...entries].sort((a, b) => a.x - b.x);
    const settings = getWorldSettings();
    const viewportRange = vpW > 0 && z > 0 ? vpW / z : 0;
    const cx = camera.x + (viewportRange > 0 ? viewportRange / 2 : 0);
    const activePosF = activePosFromCameraCentre(sortedAll, cx);
    const activeIdInWorld = sortedAll[Math.round(activePosF)]?.id ?? null;

    const factor = Number(settings.minimapZoomFactor) || 1;
    const isShowAll = factor >= MINIMAP_NEIGHBOURS_MAX;
    const pageWidth = sortedAll[0]?.width || 1280;
    const viewportPagesSpan = viewportRange > 0 && pageWidth > 0
        ? Math.max(1, viewportRange / pageWidth)
        : 1;
    const effectiveR = isShowAll
        ? sortedAll.length * 100
        : factor * Math.pow(viewportPagesSpan, 0.3);
    const layout = computeMinimapFocalLayout({
        entries: sortedAll,
        activePosF,
        focusRadius: effectiveR,
        minimapWidth,
        canvasHeight,
    });
    if (!layout) return null;

    return {
        sortedAll, layout, activeIdInWorld, activePosF,
        viewportRange, vpW, vpH, currentLayer,
        pageWidth, viewportPagesSpan, effectiveR,
    };
}

export function activePosFromCameraCentre(sortedAll, cameraCentreX) {
    if (!Array.isArray(sortedAll) || sortedAll.length === 0) return 0;
    if (sortedAll.length === 1) return 0;
    for (let i = 0; i < sortedAll.length; i++) {
        const e = sortedAll[i];
        if (cameraCentreX >= e.x && cameraCentreX < e.x + e.width) return i;
    }
    const centres = sortedAll.map(e => e.x + e.width / 2);
    if (cameraCentreX <= centres[0]) return 0;
    if (cameraCentreX >= centres[centres.length - 1]) return centres.length - 1;
    for (let i = 0; i < centres.length - 1; i++) {
        if (cameraCentreX >= centres[i] && cameraCentreX < centres[i + 1]) {
            const span = centres[i + 1] - centres[i];
            return span > 0 ? i + (cameraCentreX - centres[i]) / span : i;
        }
    }
    return centres.length - 1;
}

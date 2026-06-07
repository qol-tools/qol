import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { PageShell } from '../../components/PageShell.js';
import { PageHeader } from '../../components/PageHeader.js';
import { Surface } from '../../lib/components/Surface.js';
import {
    GHOST_OPACITY_MAX,
    GHOST_OPACITY_MIN,
    GHOST_OPACITY_STEP,
    GHOST_OPACITY_DEFAULT,
    GHOST_DEBUG_COLOR_DEFAULT,
    clampOpacity,
    formatOpacityPercent,
    isValidGhostColor,
    normalizeGhostColor,
    normalizeOpacityForServer,
    parseGpuiResponse,
} from '../../lib/runtime-gpui-opacity.js';

const ENDPOINT = '/api/dev/runtime/gpui';
const PERSIST_DEBOUNCE_MS = 150;

async function loadConfig() {
    const res = await fetch(ENDPOINT);
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const body = await res.json();
    return parseGpuiResponse(body);
}

async function persistOpacity(ghostOpacity) {
    const res = await fetch(ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ghost_opacity: normalizeOpacityForServer(ghostOpacity) }),
    });
    if (!res.ok) throw new Error('HTTP ' + res.status);
}

async function persistColor(ghostColor) {
    const res = await fetch(ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ghost_debug_color: ghostColor }),
    });
    if (!res.ok) throw new Error('HTTP ' + res.status);
}

function blurToSurface(event) {
    if (event.key !== 'Enter' && event.key !== 'Escape') return;
    event.preventDefault();
    event.stopPropagation();
    event.currentTarget.closest('[data-selected-surface]')?.focus({ preventScroll: true });
}

function OpacityRow({ value, onInput }) {
    const inputRef = useRef(null);
    const focusInput = useCallback(() => {
        inputRef.current?.focus({ preventScroll: true });
    }, []);
    return html`
        <${Surface} className="gpui-setting-row" onActivate=${focusInput}>
            <span class="gpui-setting-label">Ghost UI opacity</span>
            <input
                ref=${inputRef}
                class="gpui-setting-range"
                type="range"
                min=${GHOST_OPACITY_MIN}
                max=${GHOST_OPACITY_MAX}
                step=${GHOST_OPACITY_STEP}
                value=${value}
                onInput=${(e) => onInput(clampOpacity(e.currentTarget.value))}
                onKeyDown=${blurToSurface}
                aria-label="Ghost UI opacity"
            />
            <span class="gpui-setting-value">${formatOpacityPercent(value)}</span>
        <//>
    `;
}

function ColorRow({ value, onInput }) {
    const inputRef = useRef(null);
    const focusInput = useCallback(() => {
        inputRef.current?.focus({ preventScroll: true });
    }, []);
    const swatch = isValidGhostColor(value) ? normalizeGhostColor(value) : 'transparent';
    return html`
        <${Surface} className="gpui-setting-row" onActivate=${focusInput}>
            <span class="gpui-setting-label">Ghost debug color</span>
            <span class="gpui-setting-swatch" style=${`background:${swatch}`} aria-hidden="true"></span>
            <input
                ref=${inputRef}
                class="gpui-setting-color"
                type="text"
                inputmode="text"
                placeholder="#rrggbb"
                value=${value || ''}
                onInput=${(e) => onInput(e.currentTarget.value)}
                onKeyDown=${blurToSurface}
                aria-label="Ghost debug color hex"
            />
        <//>
    `;
}

function useDebouncedPersist(dirty, value, persist) {
    const timerRef = useRef(null);
    const [error, setError] = useState(null);
    useEffect(() => {
        if (!dirty) return undefined;
        clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => {
            persist(value).then(() => setError(null)).catch((e) => setError(e.message));
        }, PERSIST_DEBOUNCE_MS);
        return () => clearTimeout(timerRef.current);
    }, [dirty, value, persist]);
    return error;
}

export function GpuiSubPage() {
    const [ghostOpacity, setGhostOpacity] = useState(GHOST_OPACITY_DEFAULT);
    const [ghostColor, setGhostColor] = useState(GHOST_DEBUG_COLOR_DEFAULT);
    const [loaded, setLoaded] = useState(false);
    const [loadError, setLoadError] = useState(null);
    const opacityDirtyRef = useRef(false);
    const colorDirtyRef = useRef(false);

    useEffect(() => {
        let cancelled = false;
        loadConfig()
            .then((cfg) => {
                if (cancelled) return;
                setGhostOpacity(cfg.ghostOpacity);
                setGhostColor(cfg.ghostColor);
                setLoaded(true);
            })
            .catch((e) => {
                if (cancelled) return;
                setLoadError(e.message);
                setLoaded(true);
            });
        return () => { cancelled = true; };
    }, []);

    const onOpacityInput = useCallback((next) => {
        opacityDirtyRef.current = true;
        setGhostOpacity(next);
    }, []);

    const onColorInput = useCallback((next) => {
        colorDirtyRef.current = true;
        setGhostColor(next);
    }, []);

    const opacityError = useDebouncedPersist(
        loaded && opacityDirtyRef.current,
        ghostOpacity,
        persistOpacity,
    );
    const colorError = useDebouncedPersist(
        loaded && colorDirtyRef.current && (ghostColor === '' || isValidGhostColor(ghostColor)),
        ghostColor === '' ? '' : normalizeGhostColor(ghostColor),
        persistColor,
    );

    const error = loadError || opacityError || colorError;

    return html`
        <${PageShell}
            frameClassName="gpui-subpage-frame"
            header=${html`<${PageHeader} title="GPUI" subtitle="Global GPUI runtime settings applied to every plugin window" />`}>
            <div class="gpui-settings">
                <${OpacityRow} value=${ghostOpacity} onInput=${onOpacityInput} />
                <${ColorRow} value=${ghostColor} onInput=${onColorInput} />
                <p class="gpui-hint">
                    Changes apply live to running GPUI plugin windows.
                    Leave the color empty to clear the debug tint.
                </p>
                ${error && html`<p class="error-msg">${error}</p>`}
            </div>
        <//>
    `;
}

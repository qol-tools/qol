import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { Surface } from '../../../lib/components/Surface.js';
import {
    GHOST_OPACITY_MAX,
    GHOST_OPACITY_MIN,
    GHOST_OPACITY_STEP,
    GHOST_OPACITY_DEFAULT,
    clampOpacity,
    formatOpacityPercent,
    normalizeOpacityForServer,
    parseGpuiResponse,
} from '../../../lib/runtime-gpui-opacity.js';

const ENDPOINT = '/api/dev/runtime/gpui';
const PERSIST_DEBOUNCE_MS = 150;

async function loadConfig() {
    const res = await fetch(ENDPOINT);
    if (!res.ok) throw new Error('HTTP ' + res.status);
    const body = await res.json();
    return parseGpuiResponse(body);
}

async function persistConfig({ ghostOpacity }) {
    const res = await fetch(ENDPOINT, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ ghost_opacity: normalizeOpacityForServer(ghostOpacity) }),
    });
    if (!res.ok) throw new Error('HTTP ' + res.status);
}

function OpacityRangeRow({ value, onInput }) {
    const inputRef = useRef(null);
    const focusInput = useCallback(() => {
        inputRef.current?.focus({ preventScroll: true });
    }, []);
    const onKeyDown = useCallback((e) => {
        if (e.key !== 'Enter' && e.key !== 'Escape') return;
        e.preventDefault();
        e.stopPropagation();
        e.currentTarget.closest('[data-selected-surface]')?.focus({ preventScroll: true });
    }, []);
    return html`
        <${Surface} className="wsp-range-row" onActivate=${focusInput}>
            <span class="wsp-label">Ghost UI opacity</span>
            <input
                ref=${inputRef}
                class="wsp-range"
                type="range"
                min=${GHOST_OPACITY_MIN}
                max=${GHOST_OPACITY_MAX}
                step=${GHOST_OPACITY_STEP}
                value=${value}
                onInput=${(e) => onInput(clampOpacity(e.currentTarget.value))}
                onKeyDown=${onKeyDown}
                aria-label="Ghost UI opacity"
            />
            <span class="wsp-value">${formatOpacityPercent(value)}</span>
        <//>
    `;
}

function useDebouncedPersist(values) {
    const timerRef = useRef(null);
    const [error, setError] = useState(null);
    useEffect(() => {
        if (!values.dirty) return undefined;
        clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => {
            persistConfig({ ghostOpacity: values.ghostOpacity })
                .then(() => setError(null))
                .catch((e) => setError(e.message));
        }, PERSIST_DEBOUNCE_MS);
        return () => clearTimeout(timerRef.current);
    }, [values.ghostOpacity, values.dirty]);
    return error;
}

export function CoreSection() {
    const [ghostOpacity, setGhostOpacity] = useState(GHOST_OPACITY_DEFAULT);
    const [loaded, setLoaded] = useState(false);
    const [loadError, setLoadError] = useState(null);
    const dirtyRef = useRef(false);

    useEffect(() => {
        let cancelled = false;
        loadConfig()
            .then((cfg) => {
                if (cancelled) return;
                setGhostOpacity(cfg.ghostOpacity);
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
        dirtyRef.current = true;
        setGhostOpacity(next);
    }, []);

    const persistError = useDebouncedPersist({
        ghostOpacity,
        dirty: loaded && dirtyRef.current,
    });

    const error = loadError || persistError;

    return html`
        <section class="dev-section">
            <h2>Core</h2>
            <p class="dev-section-hint">
                Runtime settings that apply across plugins. Changes take effect
                next time a GPUI plugin opens its window.
            </p>
            <div class="core-runtime-group">
                <h3 class="core-runtime-group-label">GPUI</h3>
                <div class="core-runtime-grid">
                    <${OpacityRangeRow} value=${ghostOpacity} onInput=${onOpacityInput} />
                </div>
            </div>
            ${error && html`<p class="error-msg">${error}</p>`}
        </section>
    `;
}

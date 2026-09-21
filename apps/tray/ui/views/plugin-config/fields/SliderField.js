import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { isLiveNumberField } from '../../../lib/qol-config.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { Slider } from '../../../lib/components/Slider.js';
import { extractPath } from './query-data.js';
import { openColorStream, closeColorStream, streamBrightness } from './color-stream.js';
import { QOL_TRAY_INTERNAL_COLORS } from '../../../lib/generated-theme-tokens.js';

const DEFAULT_MIN = 0;
const DEFAULT_MAX = 100;
const DEFAULT_POLL_MS = 2000;

export function SliderField({ field }) {
    const ctx = usePluginConfigContext();
    const hasStream = !!field.stream;
    const isLiveNumber = isLiveNumberField(field);
    const { dispatch: sendAction, error: actionError } = useDispatchAction(ctx.pluginId, field.action || null);
    const queryDef = isLiveNumber ? ctx.runtime?.query?.[field.active_query] : null;
    const pollInterval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const activeState = useQueryPoll(ctx.pluginId, field.active_query || '', pollInterval);
    const draggingRef = useRef(false);
    const min = field.number?.min ?? field.min ?? DEFAULT_MIN;
    const max = field.number?.max ?? field.max ?? DEFAULT_MAX;
    const step = field.number?.step ?? field.step ?? 1;
    const stored = isLiveNumber ? null : ctx.getFieldValue(field);
    const [liveValue, setLiveValue] = useState(typeof field.value === 'number' ? field.value : min);
    const value = isLiveNumber ? liveValue : (typeof stored === 'number' ? stored : min);
    const gated = ctx.isRuntimeDisabled && field.stream;
    const unit = field.unit || '';

    useEffect(() => {
        if (!isLiveNumber || draggingRef.current) return;
        const polled = polledNumber(activeState.data, field.active_value_from);
        if (polled === null) return;
        setLiveValue(polled);
    }, [isLiveNumber, activeState.data, field.active_value_from]);

    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);

    const onPointerRelease = useCallback(() => {
        if (isLiveNumber) {
            draggingRef.current = false;
        }
    }, [isLiveNumber]);

    const onInput = useCallback((next) => {
        if (isLiveNumber) {
            draggingRef.current = true;
        }
        if (hasStream) {
            streamValue(field, next, ctx);
        }
    }, [isLiveNumber, hasStream, field, ctx]);

    const onCommit = useCallback((next) => {
        if (isLiveNumber) {
            draggingRef.current = false;
            setLiveValue(next);
            if (sendAction) {
                sendAction({ value: next }).catch(() => {});
            }
            return;
        }
        ctx.setFieldValue(field, next);
        ctx.saveNow().then(() => {
            if (sendAction) {
                sendAction({}).catch(() => {});
            }
        });
    }, [ctx, field, isLiveNumber, sendAction]);

    const onActiveChange = useCallback((active) => {
        if (isLiveNumber && !active) {
            draggingRef.current = false;
        }
        if (!hasStream) {
            return;
        }
        if (active) {
            openColorStream(ctx.daemonPort);
            return;
        }
        closeColorStream();
    }, [isLiveNumber, hasStream, ctx.daemonPort]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, `field-group field-slider${gated ? ' field-gated' : ''}`)}
            onMouseDown=${onSelect} onFocus=${onSelect}
            onPointerUp=${onPointerRelease} onPointerCancel=${onPointerRelease}>
            <${Slider}
                label=${field.label}
                description=${field.description || ''}
                value=${value}
                min=${min}
                max=${max}
                step=${step}
                unit=${unit}
                disabled=${gated}
                onInput=${onInput}
                onCommit=${onCommit}
                onActiveChange=${onActiveChange}
            />
            ${actionError && html`<div class="field-action-error">${actionError}</div>`}
        </div>
    `;
}

function polledNumber(data, path) {
    const value = extractPath(data, path);
    return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

function streamValue(field, value, ctx) {
    if (field.config_key === 'live_brightness') {
        const colorHex = (ctx.state?.config?.live_color_hex || QOL_TRAY_INTERNAL_COLORS.configLiveColorFallback).replace(/^#/, '');
        streamBrightness(value, colorHex);
    }
}

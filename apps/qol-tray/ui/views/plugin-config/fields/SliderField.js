import { html } from '../../../lib/html.js';
import { useCallback } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { Slider } from '../../../lib/components/Slider.js';
import { openColorStream, closeColorStream, streamBrightness } from './color-stream.js';

const DEFAULT_MIN = 0;
const DEFAULT_MAX = 100;

export function SliderField({ field }) {
    const ctx = usePluginConfigContext();
    const hasStream = !!field.stream;
    const { dispatch: sendAction } = useDispatchAction(ctx.pluginId, field.action || null);
    const stored = ctx.getFieldValue(field);
    const min = field.number?.min ?? field.min ?? DEFAULT_MIN;
    const max = field.number?.max ?? field.max ?? DEFAULT_MAX;
    const step = field.number?.step ?? field.step ?? 1;
    const value = typeof stored === 'number' ? stored : min;
    const gated = ctx.isRuntimeDisabled && field.stream;
    const unit = field.unit || '';

    const onSelect = useCallback(() => ctx.setSelectedFieldId(field.id), [ctx, field.id]);

    const onInput = useCallback((next) => {
        if (hasStream) {
            streamValue(field, next, ctx);
        }
    }, [hasStream, field, ctx]);

    const onCommit = useCallback((next) => {
        ctx.setFieldValue(field, next);
        ctx.saveNow().then(() => {
            if (sendAction) {
                sendAction().catch(() => {});
            }
        });
    }, [ctx, field, sendAction]);

    const onActiveChange = useCallback((active) => {
        if (!hasStream) {
            return;
        }
        if (active) {
            openColorStream(ctx.daemonPort);
            return;
        }
        closeColorStream();
    }, [hasStream, ctx.daemonPort]);

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, `field-group field-slider${gated ? ' field-gated' : ''}`)}
            onMouseDown=${onSelect} onFocus=${onSelect}>
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
        </div>
    `;
}

function streamValue(field, value, ctx) {
    if (field.config_key === 'live_brightness') {
        const colorHex = (ctx.state?.config?.live_color_hex || '#ffffff').replace(/^#/, '');
        streamBrightness(value, colorHex);
    }
}

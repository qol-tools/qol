import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';

export function ActionField({ field }) {
    const ctx = usePluginConfigContext();
    const { dispatch, pending, error } = useDispatchAction(ctx.pluginId, field.action);
    const [countdown, setCountdown] = useState(0);
    const intervalRef = useRef(0);

    useEffect(() => () => clearInterval(intervalRef.current), []);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const runtimeAction = ctx.runtime?.action?.[field.action];

    const run = useCallback(() => {
        if (countdown > 0 || pending) return;
        dispatch()
            .then(() => {
                const seconds = field.action === 'pair' ? 60 : 3;
                setCountdown(seconds);
                clearInterval(intervalRef.current);
                intervalRef.current = setInterval(() => {
                    setCountdown(n => {
                        if (n <= 1) {
                            clearInterval(intervalRef.current);
                            return 0;
                        }
                        return n - 1;
                    });
                }, 1000);
            })
            .catch(() => {});
    }, [dispatch, countdown, pending, field.action]);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        run();
    }, [run]);

    const variant = field.variant || 'primary';
    const exempt = field.variant === 'ghost' || field.action === 'reload' || field.action === 'pair';
    const gated = ctx.isRuntimeDisabled && !exempt;
    const busy = countdown > 0;

    const feedbackLabel = runtimeAction?.description || field.feedback_label || field.label;
    const label = busy
        ? `${feedbackLabel}... ${countdown}s`
        : pending ? 'Working...' : (field.label || 'Run');

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-action')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}
            onKeyDown=${gated ? undefined : onKeyDown}>
            <button type="button" class="btn btn-${variant}"
                    disabled=${pending || gated || busy}
                    onClick=${gated ? undefined : run}>
                ${label}
            </button>
            ${error && !busy && html`<div class="field-action-error">${error}</div>`}
        </div>
    `;
}

import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../hooks/useDispatchAction.js';
import { fieldSurfaceAttrs } from '../field-map.js';

const PAIR_DURATION_S = 60;

export function ActionField({ field }) {
    const ctx = usePluginConfigContext();
    const { dispatch, pending, error } = useDispatchAction(ctx.pluginId, field.action);
    const isPairAction = field.action === 'pair';
    const stopPair = useDispatchAction(ctx.pluginId, 'stop_pair');
    const [pairing, setPairing] = useState(false);
    const timerRef = useRef(0);

    useEffect(() => () => clearTimeout(timerRef.current), []);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const run = useCallback(() => {
        if (pending || stopPair.pending) return;

        if (isPairAction && pairing) {
            stopPair.dispatch()
                .then(() => {
                    clearTimeout(timerRef.current);
                    setPairing(false);
                })
                .catch(() => {});
            return;
        }

        dispatch()
            .then(() => {
                if (!isPairAction) return;
                setPairing(true);
                clearTimeout(timerRef.current);
                timerRef.current = setTimeout(() => setPairing(false), PAIR_DURATION_S * 1000);
            })
            .catch(() => {});
    }, [dispatch, pending, isPairAction, pairing, stopPair]);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        run();
    }, [run]);

    const variant = field.variant || 'primary';
    const exempt = field.variant === 'ghost' || field.action === 'reload' || isPairAction;
    const gated = ctx.isRuntimeDisabled && !exempt;
    const busy = pending || stopPair.pending;

    let label;
    if (isPairAction && pairing) {
        label = 'Stop Pairing';
    } else if (busy) {
        label = 'Working...';
    } else {
        label = field.label || 'Run';
    }

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-action')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}
            onKeyDown=${gated ? undefined : onKeyDown}>
            <div class="field-action-row">
                ${isPairAction && pairing && html`<span class="refresh-btn spinning"></span>`}
                <button type="button" class="btn btn-${pairing ? 'ghost' : variant}"
                        disabled=${busy || gated}
                        onClick=${gated ? undefined : run}>
                    ${label}
                </button>
            </div>
            ${error && html`<div class="field-action-error">${error}</div>`}
        </div>
    `;
}

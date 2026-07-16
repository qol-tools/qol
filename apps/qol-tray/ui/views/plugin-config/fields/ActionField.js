import { html } from '../../../lib/html.js';
import { useCallback, useEffect, useRef, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { fieldSurfaceAttrs } from '../field-map.js';
import { isActionRuntimeGated } from '../field-rules.js';
import { queryFlag } from './query-data.js';
import { actionLabel, selectedActionName } from './action-state.js';
import { Button } from '../../../lib/components/Button.js';

const PAIR_DURATION_S = 60;
const DEFAULT_POLL_MS = 2000;

export function ActionField({ field }) {
    const ctx = usePluginConfigContext();
    const primaryAction = useDispatchAction(ctx.pluginId, field.action);
    const activeAction = useDispatchAction(ctx.pluginId, field.active_action);
    const queryDef = ctx.runtime?.query?.[field.active_query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const activeState = useQueryPoll(ctx.pluginId, field.active_query, interval);
    const runtimeActive = queryFlag(activeState.data, field.active_value_from);
    const isPairAction = field.action === 'pair';
    const stopPair = useDispatchAction(ctx.pluginId, 'stop_pair');
    const [pairing, setPairing] = useState(false);
    const [syncing, setSyncing] = useState(false);
    const timerRef = useRef(0);

    useEffect(() => () => clearTimeout(timerRef.current), []);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const run = useCallback(() => {
        if (primaryAction.pending || activeAction.pending || stopPair.pending || syncing) return;

        if (field.active_action) {
            const actionName = selectedActionName(field, runtimeActive);
            const action = actionName === field.active_action ? activeAction : primaryAction;
            setSyncing(true);
            action.dispatch()
                .then(() => activeState.refresh())
                .catch(() => {})
                .finally(() => setSyncing(false));
            return;
        }

        if (isPairAction && pairing) {
            stopPair.dispatch()
                .then(() => {
                    clearTimeout(timerRef.current);
                    setPairing(false);
                })
                .catch(() => {});
            return;
        }

        primaryAction.dispatch()
            .then(() => {
                if (!isPairAction) return;
                setPairing(true);
                clearTimeout(timerRef.current);
                timerRef.current = setTimeout(() => setPairing(false), PAIR_DURATION_S * 1000);
            })
            .catch(() => {});
    }, [primaryAction, activeAction, stopPair, field.active_action, runtimeActive, isPairAction, pairing, syncing, activeState]);

    const onKeyDown = useCallback((event) => {
        if (event.key !== 'Enter' && event.key !== ' ') return;
        event.preventDefault();
        event.stopPropagation();
        run();
    }, [run]);

    const variant = field.variant || 'primary';
    const gated = isActionRuntimeGated(field, ctx.isRuntimeDisabled);
    const busy = primaryAction.pending || activeAction.pending || stopPair.pending || syncing;
    const active = runtimeActive || (isPairAction && pairing);
    const gatedMessage = gated ? 'Unavailable until the plugin connection is healthy.' : null;
    const label = actionLabel(field, busy, runtimeActive, pairing);
    const error = primaryAction.error || activeAction.error || stopPair.error || activeState.error;

    return html`
        <div ...${fieldSurfaceAttrs(field, ctx, 'field-group field-action')}
            onMouseDown=${onSelect}
            onFocus=${onSelect}
            onKeyDown=${gated ? undefined : onKeyDown}>
            <div class="field-action-row">
                ${active && html`<span class="refresh-btn spinning"></span>`}
                <${Button} type="button" variant=${`btn-${active ? 'ghost' : variant}`}
                        disabled=${busy || gated}
                        onActivate=${gated ? undefined : run}>
                    ${label}
                <//>
            </div>
            ${field.description && html`<div class="field-help">${field.description}</div>`}
            ${gatedMessage && html`<div class="field-action-error">${gatedMessage}</div>`}
            ${error && html`<div class="field-action-error">${error}</div>`}
        </div>
    `;
}

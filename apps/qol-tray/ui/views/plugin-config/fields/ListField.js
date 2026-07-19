import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { usePluginConfigContext } from '../context.js';
import { useQueryPoll } from '../../../lib/hooks/useQueryPoll.js';
import { useDispatchAction } from '../../../lib/hooks/useDispatchAction.js';
import { SearchableActionList } from '../../../lib/components/SearchableActionList.js';
import { fieldLayoutAttrs } from '../field-map.js';
import { rowActionInput } from './row-action.js';
import { listItem, rowsFrom } from './list-rows.js';
import { runtimeActivityLabel } from './query-data.js';

const DEFAULT_POLL_MS = 2000;

export function ListField({ field }) {
    const ctx = usePluginConfigContext();
    const queryDef = ctx.runtime?.query?.[field.query];
    const interval = queryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const query = useQueryPoll(ctx.pluginId, field.query, interval);
    const activeQueryDef = ctx.runtime?.query?.[field.active_query];
    const activeInterval = activeQueryDef?.poll_interval_ms || DEFAULT_POLL_MS;
    const activeState = useQueryPoll(ctx.pluginId, field.active_query, activeInterval);
    const rowDispatch = useDispatchAction(ctx.pluginId, null);
    const [pending, setPending] = useState(null);
    const rows = rowsFrom(query.data);
    const items = rows.map((row, index) => listItem(field, row, index));
    const backendPending = items.find(item => item.pending);
    const pendingId = pending?.itemId || backendPending?.id;
    const pendingActionId = pending?.actionId || backendPending?.primaryAction?.id;
    const selected = ctx.selectedFieldId === field.id;
    const activityLabel = runtimeActivityLabel(field, activeState.data);

    const onSelect = useCallback(() => {
        ctx.setSelectedFieldId(field.id);
    }, [ctx, field.id]);

    const activate = useCallback(async (item, action = item.primaryAction) => {
        if (!action?.rowAction || rowDispatch.pending) return;
        setPending({ itemId: item.id, actionId: action.id });
        try {
            await rowDispatch.dispatch(
                rowActionInput(action.rowAction, item.row),
                action.rowAction.action,
            );
            await query.refresh();
        } catch {}
        setPending(null);
    }, [rowDispatch, query]);

    return html`
        <${SearchableActionList}
            ...${fieldLayoutAttrs(field)}
            data-plugin-config-index=${ctx.fieldIndexById[field.id]}
            className="field-group field-list"
            selected=${selected ? true : undefined}
            onSelect=${onSelect}
            label=${field.label}
            statusLabel=${activityLabel}
            statusTone="success"
            statusPulse=${Boolean(activityLabel)}
            description=${field.description}
            items=${items}
            emptyMessage=${field.empty_message || 'No items.'}
            placeholder=${`Search ${field.label.toLowerCase()}...`}
            pendingId=${pendingId}
            pendingActionId=${pendingActionId}
            loading=${query.loading}
            error=${query.error || activeState.error || rowDispatch.error}
            searchable=${field.search === true}
            onActivate=${activate}
            onAction=${activate} />
    `;
}

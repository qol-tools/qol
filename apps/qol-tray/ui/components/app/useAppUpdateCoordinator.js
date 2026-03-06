import { useCallback } from 'preact/hooks';
import { useSSE, useSSEReconnect } from '../../hooks/useSSE.js';
import { clampPercent } from '../../utils/progress.js';
import { devFlowKey, devFlowPhase } from './dev-flows.js';
import { useUpdateChecker } from './use-update-checker.js';
import { useDevFlows } from './use-dev-flows.js';
import { useDevActions } from './use-dev-actions.js';

function routeDevSSE(event, applyDevFlowTransition) {
    const key = devFlowKey(event.type);
    if (!key) return;
    applyDevFlowTransition(key, devFlowPhase(event.type), event);
}

function routeUpdateSSE(event, setUpdateState, checkForUpdate) {
    if (event.type === 'update_progress') { setUpdateState({ status: 'downloading', percent: clampPercent(event.percent) }); return; }
    if (event.type === 'update_complete') { setUpdateState({ status: 'done' }); setTimeout(() => checkForUpdate(), 30000); return; }
    if (event.type === 'update_failed') setUpdateState({ status: 'error' });
}

function routeSSEEvent(event, devEnabled, applyDevFlowTransition, checkForUpdate, setUpdateState) {
    if (devEnabled) { routeDevSSE(event, applyDevFlowTransition); return; }
    routeUpdateSSE(event, setUpdateState, checkForUpdate);
}

function routeSSEReconnect(devEnabled, updateStatus, completeReconnect, checkForUpdate) {
    if (devEnabled) { completeReconnect(); return; }
    if (updateStatus === 'done') checkForUpdate();
}

export function useAppUpdateCoordinator({ devEnabled, appVersion }) {
    const { updateState, setUpdateState, checkForUpdate } = useUpdateChecker(devEnabled, appVersion);
    const devFlows = useDevFlows(setUpdateState);
    const actions = useDevActions(devEnabled, devFlows, setUpdateState);
    useSSE(useCallback(
        e => routeSSEEvent(e, devEnabled, devFlows.applyDevFlowTransition, checkForUpdate, setUpdateState),
        [devEnabled, devFlows.applyDevFlowTransition, checkForUpdate, setUpdateState]
    ));
    useSSEReconnect(useCallback(
        () => routeSSEReconnect(devEnabled, updateState.status, devFlows.completeReconnect, checkForUpdate),
        [devEnabled, updateState.status, devFlows.completeReconnect, checkForUpdate]
    ));
    return { updateState, checkForUpdate, ...actions };
}

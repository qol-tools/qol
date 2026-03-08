import { useCallback, useEffect } from 'preact/hooks';
import { useSSE, useSSEReconnect } from '../../hooks/useSSE.js';
import { clampPercent } from '../../utils/progress.js';
import { devFlowKey, devFlowPhase } from './dev-flows.js';
import { useUpdateChecker } from './use-update-checker.js';
import { useDevFlows } from './use-dev-flows.js';
import { useDevActions } from './use-dev-actions.js';
import { MOCK_FLOWS_DONE } from '../../views/dev/mock/events.js';

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
    if (devEnabled) return completeReconnect();
    if (updateStatus === 'downloading') return 'update';
    if (updateStatus === 'done') { checkForUpdate(); return null; }
    return null;
}

export function useAppUpdateCoordinator({ devEnabled, appVersion, onDissolve }) {
    const { updateState, setUpdateState, checkForUpdate } = useUpdateChecker(devEnabled, appVersion);
    const devFlows = useDevFlows(setUpdateState);
    const actions = useDevActions(devEnabled, devFlows, setUpdateState);
    useSSE(useCallback(
        e => routeSSEEvent(e, devEnabled, devFlows.applyDevFlowTransition, checkForUpdate, setUpdateState),
        [devEnabled, devFlows.applyDevFlowTransition, checkForUpdate, setUpdateState]
    ));
    useSSEReconnect(useCallback(
        () => {
            const restartedFlow = routeSSEReconnect(devEnabled, updateState.status, devFlows.completeReconnect, checkForUpdate);
            if (restartedFlow && onDissolve) onDissolve(true);
        },
        [devEnabled, updateState.status, devFlows.completeReconnect, checkForUpdate, onDissolve]
    ));
    useEffect(() => {
        const handler = () => { if (onDissolve) onDissolve(false); };
        document.addEventListener(MOCK_FLOWS_DONE, handler);
        return () => document.removeEventListener(MOCK_FLOWS_DONE, handler);
    }, [onDissolve]);
    return { updateState, checkForUpdate, ...actions };
}

import { useCallback, useEffect } from 'preact/hooks';
import { useSSE, useSSEReconnect } from '../hooks/useSSE.js';
import { useUpdateChecker } from './use-update-checker.js';
import { useDevFlows } from './use-dev-flows.js';
import { useDevActions } from './use-dev-actions.js';
import { MOCK_FLOWS_DONE } from '../views/dev/mock/events.js';
import { routeSSEEvent, routeSSEReconnect } from './update-sse-routing.js';
import { setHostRestarting, statusImpliesRestart } from '../lib/host-restart.js';

export function useAppUpdateCoordinator({ devEnabled, appVersion, onDissolve }) {
    const { updateState, setUpdateState, checkForUpdate } = useUpdateChecker(devEnabled, appVersion);
    const devFlows = useDevFlows(setUpdateState);
    const actions = useDevActions(devEnabled, devFlows, setUpdateState);
    useSSE(useCallback(
        e => routeSSEEvent(e, devEnabled, devFlows.applyDevFlowTransition, setUpdateState),
        [devEnabled, devFlows.applyDevFlowTransition, setUpdateState]
    ));
    useSSEReconnect(useCallback(
        () => {
            const restartedFlow = routeSSEReconnect(devEnabled, updateState.status, devFlows.completeReconnect);
            if (restartedFlow && onDissolve) onDissolve(true);
        },
        [devEnabled, updateState.status, devFlows.completeReconnect, onDissolve]
    ));
    useEffect(() => {
        const handler = () => { if (onDissolve) onDissolve(false); };
        document.addEventListener(MOCK_FLOWS_DONE, handler);
        return () => document.removeEventListener(MOCK_FLOWS_DONE, handler);
    }, [onDissolve]);
    useEffect(() => {
        setHostRestarting(statusImpliesRestart(updateState.status));
    }, [updateState.status]);
    return { updateState, checkForUpdate, ...actions };
}

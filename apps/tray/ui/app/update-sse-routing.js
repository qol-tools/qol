import { clampPercent } from '../utils/progress.js';
import { devFlowKey, devFlowPhase } from './dev-flows.js';
import { applyNavigateRoute } from '../lib/deeplink-navigate.js';

function routeDevSSE(event, applyDevFlowTransition) {
    const key = devFlowKey(event.type);
    if (!key) return;
    applyDevFlowTransition(key, devFlowPhase(event.type), event);
}

export function routeUpdateSSE(event, setUpdateState) {
    if (event.type === 'update_progress') { setUpdateState({ status: 'downloading', percent: clampPercent(event.percent) }); return; }
    if (event.type === 'update_complete') { setUpdateState({ status: 'done' }); return; }
    if (event.type === 'update_failed') setUpdateState({ status: 'error' });
}

export function routeSSEEvent(event, devEnabled, applyDevFlowTransition, setUpdateState) {
    if (event.type === 'navigate' && typeof event.route === 'string') { applyNavigateRoute(event.route); return; }
    if (devEnabled) { routeDevSSE(event, applyDevFlowTransition); return; }
    routeUpdateSSE(event, setUpdateState);
}

export function routeSSEReconnect(devEnabled, updateStatus, completeReconnect) {
    if (devEnabled) return completeReconnect();
    if (updateStatus === 'downloading' || updateStatus === 'done') return 'update';
    return null;
}

import { useRef, useCallback } from 'preact/hooks';
import {
    applyDevFlowTransition as applyFlowStateTransition,
    completeReconnectFlows,
    FLOW_STATE,
    initDevFlows,
    resolveDevSidebarState,
    startRecompileFlow,
    startUpdateFlow
} from './dev-flows.js';

function doClearTimer(flows, key) {
    const flow = flows[key];
    if (!flow?.clearTimer) return;
    clearTimeout(flow.clearTimer);
    flow.clearTimer = null;
}

function doScheduleClear(flows, syncSidebar, key, ms) {
    doClearTimer(flows, key);
    const flow = flows[key];
    if (!flow) return;
    flow.clearTimer = setTimeout(() => {
        flow.clearTimer = null;
        if (flow.state === FLOW_STATE.DONE) {
            flow.state = FLOW_STATE.IDLE;
            flow.percent = 0;
            flow.phase = null;
            flow.message = null;
            flow.restarts = false;
            syncSidebar();
        }
    }, ms);
}

function doApplyTransition(devFlowsRef, key, phase, event, syncSidebar) {
    doClearTimer(devFlowsRef.current, key);
    applyFlowStateTransition(devFlowsRef.current, { key, phase, event }, (k, ms) => doScheduleClear(devFlowsRef.current, syncSidebar, k, ms));
    syncSidebar();
}

function doBeginUpdateFlow(devFlowsRef, syncSidebar) {
    doClearTimer(devFlowsRef.current, 'update');
    startUpdateFlow(devFlowsRef.current);
    syncSidebar();
}

function doBeginRecompileFlow(devFlowsRef, syncSidebar) {
    const flows = devFlowsRef.current;
    if (flows.recompile.state === FLOW_STATE.ACTIVE || flows.update.state === FLOW_STATE.ACTIVE) return false;
    doClearTimer(flows, 'recompile');
    startRecompileFlow(flows);
    syncSidebar();
    return true;
}

export function useDevFlows(setUpdateState) {
    const devFlowsRef = useRef(initDevFlows());
    const syncSidebar = useCallback(() => setUpdateState(resolveDevSidebarState(devFlowsRef.current)), [setUpdateState]);
    const applyDevFlowTransition = useCallback((key, phase, event) => doApplyTransition(devFlowsRef, key, phase, event, syncSidebar), [syncSidebar]);
    const beginUpdateFlow = useCallback(() => doBeginUpdateFlow(devFlowsRef, syncSidebar), [syncSidebar]);
    const beginRecompileFlow = useCallback(() => doBeginRecompileFlow(devFlowsRef, syncSidebar), [syncSidebar]);
    const completeReconnect = useCallback(() => {
        const restartedFlow = completeReconnectFlows(devFlowsRef.current, (k, ms) => doScheduleClear(devFlowsRef.current, syncSidebar, k, ms));
        syncSidebar();
        return restartedFlow;
    }, [syncSidebar]);
    return { applyDevFlowTransition, beginUpdateFlow, beginRecompileFlow, completeReconnect };
}

import { clampPercent } from '../../utils/progress.js';

export const FLOW_STATE = {
    IDLE: 'idle',
    ACTIVE: 'active',
    RESTARTING: 'restarting',
    DONE: 'done',
    FAILED: 'failed'
};

const DEV_FLOW_CLEAR_MS = {
    recompile: 1800,
    update: 2000
};

const DEV_FLOW_EVENT_KEYS = {
    self_recompile_progress: 'recompile',
    self_recompile_complete: 'recompile',
    self_recompile_failed: 'recompile',
    update_progress: 'update',
    update_complete: 'update',
    update_failed: 'update'
};

function idleFlow() {
    return { state: FLOW_STATE.IDLE, percent: 0, phase: null, message: null, restarts: false, clearTimer: null };
}

export function initDevFlows() {
    return {
        update: idleFlow(),
        recompile: idleFlow()
    };
}

export function resolveDevSidebarState(devFlows) {
    const { recompile, update } = devFlows;
    if (recompile.state === FLOW_STATE.FAILED) return { status: 'error', message: recompile.message };
    if (recompile.state === FLOW_STATE.ACTIVE) return { status: 'compiling', percent: recompile.percent, phase: recompile.phase || 'Recompiling QoL Tray' };
    if (recompile.state === FLOW_STATE.RESTARTING) return { status: 'compiling', percent: 100, phase: 'Restarting...' };
    if (recompile.state === FLOW_STATE.DONE) return { status: 'recompile_done' };
    if (update.state === FLOW_STATE.FAILED) return { status: 'error', message: update.message };
    if (update.state === FLOW_STATE.ACTIVE) return { status: 'downloading', percent: update.percent };
    if (update.state === FLOW_STATE.RESTARTING) return { status: 'downloading', percent: 100 };
    if (update.state === FLOW_STATE.DONE) return { status: 'done' };
    return { status: 'idle' };
}

export function devFlowKey(type) {
    return DEV_FLOW_EVENT_KEYS[type] || null;
}

export function devFlowPhase(type) {
    if (type.endsWith('_progress')) return 'progress';
    if (type.endsWith('_complete')) return 'complete';
    return 'failed';
}

export function applyDevFlowTransition(devFlows, transition, scheduleDoneClear) {
    const { key, phase, event } = transition;
    const flow = devFlows[key];
    if (phase === 'progress') {
        flow.state = FLOW_STATE.ACTIVE;
        flow.percent = clampPercent(event.percent);
        flow.phase = (key === 'recompile') ? progressPhase(event) : null;
        flow.message = null;
        return;
    }
    if (phase === 'complete') {
        const nextState = flow.restarts ? FLOW_STATE.RESTARTING : FLOW_STATE.DONE;
        flow.state = nextState;
        flow.percent = 100;
        flow.phase = null;
        flow.message = null;
        if (nextState === FLOW_STATE.DONE) scheduleDoneClear(key, DEV_FLOW_CLEAR_MS[key]);
        return;
    }
    flow.state = FLOW_STATE.FAILED;
    flow.percent = 0;
    flow.phase = null;
    flow.message = event?.message || `${key} failed`;
}

export function startUpdateFlow(devFlows) {
    devFlows.update = { state: FLOW_STATE.ACTIVE, percent: 0, phase: null, message: null, restarts: true, clearTimer: null };
}

export function startRecompileFlow(devFlows) {
    devFlows.recompile = { state: FLOW_STATE.ACTIVE, percent: 0, phase: 'Preparing build', message: null, restarts: true, clearTimer: null };
}

export function completeReconnectFlows(devFlows, scheduleDoneClear) {
    let restartedFlow = null;
    for (const key of ['recompile', 'update']) {
        const flow = devFlows[key];
        if (flow.state !== FLOW_STATE.RESTARTING) continue;
        restartedFlow = key;
        flow.state = FLOW_STATE.DONE;
        flow.restarts = false;
        scheduleDoneClear(key, DEV_FLOW_CLEAR_MS[key]);
    }
    return restartedFlow;
}

function progressPhase(event) {
    if (typeof event.phase !== 'string') return 'Recompiling QoL Tray';
    if (!event.phase.trim()) return 'Recompiling QoL Tray';
    return event.phase;
}

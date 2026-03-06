import { clampPercent } from '../../utils/progress.js';

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

export function initDevFlows() {
    return {
        update: {
            active: false,
            percent: 0,
            done: false,
            error: null,
            clearTimer: null
        },
        recompile: {
            active: false,
            percent: 0,
            phase: 'Preparing build',
            done: false,
            error: null,
            clearTimer: null
        }
    };
}

export function resolveDevSidebarState(devFlows) {
    const { recompile, update } = devFlows;
    if (recompile.error) return { status: 'error', message: recompile.error };
    if (recompile.active) return { status: 'compiling', percent: recompile.percent, phase: recompile.phase || 'Recompiling QoL Tray' };
    if (update.error) return { status: 'error', message: update.error };
    if (update.active) return { status: 'downloading', percent: update.percent };
    if (recompile.done) return { status: 'recompile_done' };
    if (update.done) return { status: 'done' };
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
    if (phase === 'progress') {
        applyProgress(devFlows[key], key, event);
        return;
    }
    if (phase === 'complete') {
        applyComplete(devFlows[key], key, scheduleDoneClear);
        return;
    }

    applyFailure(devFlows[key], key, event);
}

export function startUpdateFlow(devFlows) {
    devFlows.update = {
        active: true,
        percent: 0,
        done: false,
        error: null,
        clearTimer: null
    };
}

export function startRecompileFlow(devFlows) {
    devFlows.recompile = {
        active: true,
        percent: 0,
        phase: 'Preparing build',
        done: false,
        error: null,
        clearTimer: null
    };
}

export function completeReconnectFlows(devFlows, applyTransition) {
    if (devFlows.recompile.active) {
        applyTransition('recompile', 'complete', {});
    }
    if (devFlows.update.active) {
        applyTransition('update', 'complete', {});
    }
}

function applyProgress(flow, key, event) {
    flow.active = true;
    flow.percent = clampPercent(event.percent);
    if (key === 'recompile') {
        flow.phase = progressPhase(event);
    }
    flow.done = false;
    flow.error = null;
}

function applyComplete(flow, key, scheduleDoneClear) {
    flow.active = false;
    flow.percent = 100;
    flow.done = true;
    flow.error = null;
    scheduleDoneClear(key, DEV_FLOW_CLEAR_MS[key]);
}

function applyFailure(flow, key, event) {
    flow.active = false;
    flow.done = false;
    flow.error = event?.message || `${key} failed`;
}

function progressPhase(event) {
    if (typeof event.phase !== 'string') {
        return 'Recompiling QoL Tray';
    }
    if (!event.phase.trim()) {
        return 'Recompiling QoL Tray';
    }
    return event.phase;
}

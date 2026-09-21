const RESTART_PENDING = new Set(['downloading', 'compiling', 'done', 'recompile_done']);

let restarting = false;

export function statusImpliesRestart(status) {
    return RESTART_PENDING.has(status);
}

export function setHostRestarting(value) {
    restarting = value;
}

export function isHostRestarting() {
    return restarting;
}

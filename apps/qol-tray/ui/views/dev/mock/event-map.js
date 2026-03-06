const EVENT_TO_TARGET = {
    update_complete: 'self_update',
    update_failed: 'self_update',
    self_recompile_complete: 'self_recompile',
    self_recompile_failed: 'self_recompile',
    build_complete: 'plugin_build'
};

export function mockTargetForEvent(eventType) {
    return EVENT_TO_TARGET[eventType] ?? null;
}

export function firstEnabledActionIndex(actions) {
    return actions.findIndex(action => !action.disabled);
}

export function lastEnabledActionIndex(actions) {
    for (let index = actions.length - 1; index >= 0; index -= 1) {
        if (!actions[index].disabled) return index;
    }
    return -1;
}

export function nextEnabledActionIndex(actions, currentIndex, direction) {
    if (actions.length === 0) return -1;
    const fallback = direction < 0
        ? lastEnabledActionIndex(actions)
        : firstEnabledActionIndex(actions);
    if (currentIndex < 0) return fallback;
    for (let offset = 1; offset <= actions.length; offset += 1) {
        const index = (currentIndex + direction * offset + actions.length) % actions.length;
        if (!actions[index].disabled) return index;
    }
    return -1;
}

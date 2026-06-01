export function computeFrameProgress({
    buildAnimation,
    current,
    target,
    timestamp,
    lastFrameTime
}) {
    const delta = target - current;
    const allowHardSync = target < (buildAnimation.completionTriggerPercent - 8);
    if (allowHardSync && delta > buildAnimation.hardSyncDelta) {
        return {
            percent: target,
            mode: 'sync'
        };
    }
    if (Math.abs(delta) <= buildAnimation.snapDelta) {
        return {
            percent: target,
            mode: 'sync'
        };
    }

    const elapsed = lastFrameTime > 0 ? timestamp - lastFrameTime : 16;
    const dt = Math.min(buildAnimation.frameMaxMs, Math.max(buildAnimation.frameMinMs, elapsed));
    const alpha = 1 - Math.exp(-dt / buildAnimation.easeMs);
    const eased = current + delta * alpha;

    return {
        percent: Math.max(current, eased),
        mode: 'animate'
    };
}

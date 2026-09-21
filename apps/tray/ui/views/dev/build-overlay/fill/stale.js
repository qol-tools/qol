export function resetStaleProgressState({
    rowRef,
    status,
    normalizedPercent,
    staleResetPercent,
    stopFillAnimation
}) {
    if (status !== 'queued' && status !== 'building') {
        return;
    }
    if (normalizedPercent > staleResetPercent) {
        return;
    }
    if (!Number.isFinite(rowRef.lastBuildPercent) && !Number.isFinite(rowRef.displayPercent)) {
        return;
    }

    rowRef.displayPercent = Number.NaN;
    rowRef.targetPercent = Number.NaN;
    rowRef.lastBuildPercent = Number.NaN;
    rowRef.lastFrameTime = 0;
    stopFillAnimation(rowRef);
}

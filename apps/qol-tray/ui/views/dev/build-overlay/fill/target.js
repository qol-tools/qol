export function createFillTargetController({
    normalizePercent,
    applyFillScale,
    stopFillAnimation,
    queueFillAnimation
}) {
    function setFillTarget(rowRef, targetPercent, immediate) {
        const nextPercent = normalizePercent(targetPercent);
        rowRef.lastBuildPercent = nextPercent;
        if (!rowRef.fill || rowRef.completing) {
            return;
        }

        if (immediate) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent, normalizePercent);
            stopFillAnimation(rowRef);
            return;
        }

        if (!Number.isFinite(rowRef.displayPercent)) {
            rowRef.displayPercent = 0;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, 0, normalizePercent);
            rowRef.lastFrameTime = performance.now();
            queueFillAnimation(rowRef);
            return;
        }

        const delta = Math.abs(nextPercent - rowRef.displayPercent);
        if (delta <= 0.01) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent, normalizePercent);
            stopFillAnimation(rowRef);
            return;
        }

        rowRef.targetPercent = nextPercent;
        rowRef.lastFrameTime = performance.now();
        queueFillAnimation(rowRef);
    }

    return {
        setFillTarget
    };
}

export function createFillController({
    buildAnimation,
    normalizePercent,
    applyFillScale,
    finiteOr
}) {
    function toDisplayPercent(rowRef, normalizedPercent, status) {
        if (status !== 'building') {
            return normalizedPercent;
        }
        if (!Number.isFinite(rowRef.lastBuildPercent)) {
            return normalizedPercent;
        }
        if (normalizedPercent >= rowRef.lastBuildPercent) {
            return normalizedPercent;
        }
        return rowRef.lastBuildPercent;
    }

    function resetStaleProgressState(rowRef, status, normalizedPercent) {
        if (status !== 'queued' && status !== 'building') {
            return;
        }
        if (normalizedPercent > buildAnimation.staleResetPercent) {
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

    function stopFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            cancelAnimationFrame(rowRef.animationFrame);
            rowRef.animationFrame = null;
        }
        rowRef.lastFrameTime = 0;
    }

    return {
        resetStaleProgressState,
        setFillTarget,
        stopFillAnimation,
        toDisplayPercent
    };

    function queueFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            return;
        }

        rowRef.animationFrame = requestAnimationFrame(timestamp => animateFill(rowRef, timestamp));
    }

    function animateFill(rowRef, timestamp) {
        rowRef.animationFrame = null;
        if (!rowRef.fill) {
            return;
        }

        const current = finiteOr(rowRef.displayPercent, 0);
        const target = finiteOr(rowRef.targetPercent, current);
        const delta = target - current;
        const allowHardSync = target < (buildAnimation.completionTriggerPercent - 8);
        if (allowHardSync && delta > buildAnimation.hardSyncDelta) {
            syncFill(rowRef, target, timestamp);
            return;
        }
        if (Math.abs(delta) <= buildAnimation.snapDelta) {
            syncFill(rowRef, target, timestamp);
            return;
        }

        const elapsed = rowRef.lastFrameTime > 0 ? timestamp - rowRef.lastFrameTime : 16;
        const dt = Math.min(buildAnimation.frameMaxMs, Math.max(buildAnimation.frameMinMs, elapsed));
        rowRef.lastFrameTime = timestamp;
        const alpha = 1 - Math.exp(-dt / buildAnimation.easeMs);
        const eased = current + delta * alpha;
        const next = Math.max(current, eased);

        rowRef.displayPercent = next;
        applyFillScale(rowRef, next, normalizePercent);
        rowRef.animationFrame = requestAnimationFrame(nextTimestamp => animateFill(rowRef, nextTimestamp));
    }

    function syncFill(rowRef, percent, timestamp) {
        rowRef.displayPercent = percent;
        applyFillScale(rowRef, percent, normalizePercent);
        rowRef.lastFrameTime = timestamp;
    }
}

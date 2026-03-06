export function createCompletionPhaseRenderer({
    buildAnimation,
    normalizePercent,
    applyFillScale,
    finiteOr
}) {
    function snapshot(completion, now) {
        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'fade') {
            return { percent: 100, completing: true };
        }
        if (phase === 'hold') {
            return { percent: 100, completing: false };
        }

        const rampMs = buildAnimation.completionRampMs;
        const startPercent = normalizePercent(completion.startPercent);
        const t = rampMs <= 0 ? 1 : Math.max(0, Math.min(1, phaseElapsed / rampMs));
        const eased = easeOutCubic(t);
        return {
            percent: startPercent + (100 - startPercent) * eased,
            completing: false
        };
    }

    function renderFrame(rowRef, completion, timestamp) {
        const phase = completion.phase || 'ramp';
        if (phase === 'ramp') {
            return renderRamp(rowRef, completion, timestamp);
        }
        if (phase === 'hold') {
            return renderHold(rowRef, completion, timestamp);
        }
        return renderFade(rowRef, completion, timestamp);
    }

    function remainingMs(completion, now) {
        const phase = completion.phase || 'ramp';
        const phaseStartedAt = finiteOr(completion.phaseStartedAt, now);
        const phaseElapsed = Math.max(0, now - phaseStartedAt);
        if (phase === 'ramp') {
            const rampRemaining = Math.max(0, buildAnimation.completionRampMs - phaseElapsed);
            return rampRemaining + buildAnimation.completionHoldMs + buildAnimation.completionVisibleMs;
        }
        if (phase === 'hold') {
            const holdRemaining = Math.max(0, buildAnimation.completionHoldMs - phaseElapsed);
            return holdRemaining + buildAnimation.completionVisibleMs;
        }
        return Math.max(0, buildAnimation.completionVisibleMs - phaseElapsed);
    }

    return {
        remainingMs,
        renderFrame,
        snapshot
    };

    function renderRamp(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        const rampMs = buildAnimation.completionRampMs;
        if (phaseElapsed < rampMs) {
            const progressT = rampMs <= 0 ? 1 : phaseElapsed / rampMs;
            const eased = easeOutCubic(progressT);
            const startPercent = normalizePercent(completion.startPercent);
            const nextPercent = startPercent + (100 - startPercent) * eased;
            applyProgress(rowRef, nextPercent, false);
            return false;
        }

        completion.phase = 'hold';
        completion.phaseStartedAt = timestamp;
        applyProgress(rowRef, 100, false);
        return false;
    }

    function renderHold(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyProgress(rowRef, 100, false);
        if (phaseElapsed < buildAnimation.completionHoldMs) {
            return false;
        }

        completion.phase = 'fade';
        completion.phaseStartedAt = timestamp;
        applyProgress(rowRef, 100, true);
        return false;
    }

    function renderFade(rowRef, completion, timestamp) {
        const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
        applyProgress(rowRef, 100, true);
        return phaseElapsed >= buildAnimation.completionVisibleMs;
    }

    function applyProgress(rowRef, percent, completing) {
        rowRef.completing = completing;
        if (rowRef.overlay) {
            rowRef.overlay.classList.toggle('is-completing', completing);
        }
        rowRef.displayPercent = percent;
        rowRef.targetPercent = 100;
        rowRef.lastBuildPercent = 100;
        applyFillScale(rowRef, percent, normalizePercent);
    }
}

function easeOutCubic(value) {
    const clamped = Math.max(0, Math.min(1, value));
    const inverse = 1 - clamped;
    return 1 - inverse * inverse * inverse;
}

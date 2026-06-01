function easeOutCubic(value) {
    const clamped = Math.max(0, Math.min(1, value));
    const inverse = 1 - clamped;
    return 1 - inverse * inverse * inverse;
}

function applyProgressImpl(rowRef, percent, completing, applyFillScale, normalizePercent) {
    rowRef.completing = completing;
    if (rowRef.overlay) {
        rowRef.overlay.classList.toggle('is-completing', completing);
    }
    rowRef.displayPercent = percent;
    rowRef.targetPercent = 100;
    rowRef.lastBuildPercent = 100;
    applyFillScale(rowRef, percent, normalizePercent);
}

function renderRampImpl(rowRef, completion, timestamp, buildAnimation, finiteOr, normalizePercent, applyProg) {
    const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
    const rampMs = buildAnimation.completionRampMs;
    if (phaseElapsed < rampMs) {
        const progressT = rampMs <= 0 ? 1 : phaseElapsed / rampMs;
        const eased = easeOutCubic(progressT);
        const startPercent = normalizePercent(completion.startPercent);
        const nextPercent = startPercent + (100 - startPercent) * eased;
        applyProg(rowRef, nextPercent, false);
        return false;
    }
    completion.phase = 'hold';
    completion.phaseStartedAt = timestamp;
    applyProg(rowRef, 100, false);
    return false;
}

function renderHoldImpl(rowRef, completion, timestamp, buildAnimation, finiteOr, applyProg) {
    const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
    applyProg(rowRef, 100, false);
    if (phaseElapsed < buildAnimation.completionHoldMs) {
        return false;
    }
    completion.phase = 'fade';
    completion.phaseStartedAt = timestamp;
    applyProg(rowRef, 100, true);
    return false;
}

function renderFadeImpl(rowRef, completion, timestamp, buildAnimation, finiteOr, applyProg) {
    const phaseElapsed = Math.max(0, timestamp - finiteOr(completion.phaseStartedAt, timestamp));
    applyProg(rowRef, 100, true);
    return phaseElapsed >= buildAnimation.completionVisibleMs;
}

function snapshotImpl(completion, now, buildAnimation, normalizePercent, finiteOr) {
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

function remainingMsImpl(completion, now, buildAnimation, finiteOr) {
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

export function createCompletionPhaseRenderer({ buildAnimation, normalizePercent, applyFillScale, finiteOr }) {
    const applyProg = (rowRef, pct, completing) => applyProgressImpl(rowRef, pct, completing, applyFillScale, normalizePercent);
    const ramp = (rowRef, comp, ts) => renderRampImpl(rowRef, comp, ts, buildAnimation, finiteOr, normalizePercent, applyProg);
    const hold = (rowRef, comp, ts) => renderHoldImpl(rowRef, comp, ts, buildAnimation, finiteOr, applyProg);
    const fade = (rowRef, comp, ts) => renderFadeImpl(rowRef, comp, ts, buildAnimation, finiteOr, applyProg);
    return {
        remainingMs: (comp, now) => remainingMsImpl(comp, now, buildAnimation, finiteOr),
        renderFrame: (rowRef, comp, ts) => {
            const phase = comp.phase || 'ramp';
            if (phase === 'ramp') return ramp(rowRef, comp, ts);
            if (phase === 'hold') return hold(rowRef, comp, ts);
            return fade(rowRef, comp, ts);
        },
        snapshot: (comp, now) => snapshotImpl(comp, now, buildAnimation, normalizePercent, finiteOr),
    };
}

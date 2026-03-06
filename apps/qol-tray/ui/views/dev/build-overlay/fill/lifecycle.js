import { computeFrameProgress } from './progression.js';

export function createAnimationLifecycle({
    buildAnimation,
    normalizePercent,
    applyFillScale,
    finiteOr
}) {
    function stopFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            cancelAnimationFrame(rowRef.animationFrame);
            rowRef.animationFrame = null;
        }
        rowRef.lastFrameTime = 0;
    }

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
        const frame = computeFrameProgress({
            buildAnimation,
            current,
            target,
            timestamp,
            lastFrameTime: rowRef.lastFrameTime
        });

        rowRef.displayPercent = frame.percent;
        applyFillScale(rowRef, frame.percent, normalizePercent);
        rowRef.lastFrameTime = timestamp;
        if (frame.mode !== 'animate') {
            return;
        }

        rowRef.animationFrame = requestAnimationFrame(nextTimestamp => animateFill(rowRef, nextTimestamp));
    }

    return {
        queueFillAnimation,
        stopFillAnimation
    };
}

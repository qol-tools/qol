import { toDisplayPercent } from './fill/display.js';
import { createAnimationLifecycle } from './fill/lifecycle.js';
import { resetStaleProgressState } from './fill/stale.js';
import { createFillTargetController } from './fill/target.js';

export function createFillController({
    buildAnimation,
    normalizePercent,
    applyFillScale,
    finiteOr
}) {
    const animationLifecycle = createAnimationLifecycle({
        buildAnimation,
        normalizePercent,
        applyFillScale,
        finiteOr
    });
    const targetController = createFillTargetController({
        normalizePercent,
        applyFillScale,
        stopFillAnimation: animationLifecycle.stopFillAnimation,
        queueFillAnimation: animationLifecycle.queueFillAnimation
    });

    return {
        resetStaleProgressState(rowRef, status, normalizedPercent) {
            resetStaleProgressState({
                rowRef,
                status,
                normalizedPercent,
                staleResetPercent: buildAnimation.staleResetPercent,
                stopFillAnimation: animationLifecycle.stopFillAnimation
            });
        },
        setFillTarget: targetController.setFillTarget,
        stopFillAnimation: animationLifecycle.stopFillAnimation,
        toDisplayPercent
    };
}

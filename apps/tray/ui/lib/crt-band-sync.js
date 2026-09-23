const SYNCED_ANIMATIONS = new Set(['crt-band-roll', 'crt-band-breathe']);

export function installCrtBandSync() {
    const pinToGlobalPhase = (event) => {
        if (!SYNCED_ANIMATIONS.has(event.animationName)) return;
        if (!(event.target instanceof Element)) return;
        for (const animation of event.target.getAnimations({ subtree: true })) {
            if (SYNCED_ANIMATIONS.has(animation.animationName) && animation.startTime !== 0) {
                animation.startTime = 0;
            }
        }
    };
    document.addEventListener('animationstart', pinToGlobalPhase);
    return () => document.removeEventListener('animationstart', pinToGlobalPhase);
}

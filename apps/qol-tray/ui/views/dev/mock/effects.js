export function createMockTargetsPoller({ onTick, intervalMs = 800 }) {
    let timer = null;

    function stop() {
        if (!timer) return;
        clearInterval(timer);
        timer = null;
    }

    function start() {
        stop();
        timer = setInterval(() => {
            void onTick();
        }, intervalMs);
        void onTick();
    }

    return { start, stop };
}

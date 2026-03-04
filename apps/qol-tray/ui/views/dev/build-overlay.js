import { BUILD_ANIMATION } from './build-animation.js';

export function createPluginBuildOverlayController({
    getContainer,
    getPluginById,
    getBuildState,
    formatDetail,
    normalizePercent
}) {
    const LOG_PREFIX = '[qol-dev-loading]';
    const DEBUG_LOADING = false;
    let rowRefs = new Map();
    const completionPlayedByPlugin = new Set();
    const pendingBuildRows = new Set();
    let buildSyncFrame = null;

    function log(event, payload) {
        if (!DEBUG_LOADING) return;
        console.info(`${LOG_PREFIX} ${event}`, payload);
    }

    function clearQueued() {
        pendingBuildRows.clear();
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
        for (const rowRef of rowRefs.values()) {
            stopFillAnimation(rowRef);
            stopCompletion(rowRef);
        }
    }

    function completeRows(pluginIds, onComplete) {
        const done = typeof onComplete === 'function' ? onComplete : () => {};
        let pending = 0;
        let finished = false;

        const flush = () => {
            if (finished) return;
            if (pending > 0) return;
            finished = true;
            done();
        };

        for (const pluginId of pluginIds) {
            const rowRef = rowRefs.get(pluginId);
            if (!rowRef) continue;
            if (completionPlayedByPlugin.has(rowRef.pluginId) && !rowRef.completing) {
                clearOverlayNodes(rowRef);
                continue;
            }
            const started = startCompletion(rowRef, true, () => {
                pending = Math.max(0, pending - 1);
                flush();
            });
            if (!started) continue;
            pending += 1;
        }

        flush();
        return pending > 0;
    }

    function queue(pluginId, onNeedsFullRender) {
        if (!pluginId) return;
        log('overlay:queue', { pluginId });
        pendingBuildRows.add(pluginId);
        if (buildSyncFrame !== null) return;

        buildSyncFrame = requestAnimationFrame(() => {
            buildSyncFrame = null;
            let needsFullRender = false;
            for (const queuedId of pendingBuildRows) {
                if (!syncRow(queuedId)) {
                    needsFullRender = true;
                    break;
                }
            }
            pendingBuildRows.clear();
            if (needsFullRender && typeof onNeedsFullRender === 'function') {
                onNeedsFullRender();
            }
        });
    }

    function cacheRows() {
        const previousRows = rowRefs;
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
        for (const rowRef of previousRows.values()) {
            stopFillAnimation(rowRef);
        }
        rowRefs = new Map();
        const container = getContainer();
        if (!container) return;

        const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
        for (const row of rows) {
            const pluginId = row.dataset.pluginId;
            if (!pluginId) continue;
            const previous = previousRows.get(pluginId);
            rowRefs.set(pluginId, makeRowRef(row, previous));
        }
        log('overlay:cache-rows', {
            rowCount: rows.length,
            mappedCount: rowRefs.size,
            pluginIds: Array.from(rowRefs.keys())
        });
    }

    function makeRowRef(row, previous) {
        return {
            row,
            pluginId: row.dataset.pluginId || '',
            overlayHost: row.querySelector('.plugin-build-overlay-host'),
            overlay: null,
            fill: null,
            main: null,
            sub: null,
            displayPercent: finiteOr(previous?.displayPercent, Number.NaN),
            targetPercent: finiteOr(previous?.targetPercent, Number.NaN),
            lastBuildPercent: finiteOr(previous?.lastBuildPercent, Number.NaN),
            animationFrame: null,
            completionTimer: null,
            completionFrame: null,
            completing: false,
            completionWaiters: [],
            lastFrameTime: 0,
            lastMain: '',
            lastSub: ''
        };
    }

    function syncAll(pluginIds) {
        for (const pluginId of pluginIds) {
            syncRow(pluginId);
        }
    }

    function syncRow(pluginId) {
        if (!getContainer()) return false;
        const rowRef = rowRefs.get(pluginId);
        if (!rowRef) return false;
        const plugin = getPluginById(pluginId);
        if (!plugin) return false;

        const buildState = getBuildState(plugin);
        const isBuilding = !!buildState;
        rowRef.row.classList.toggle('is-building', isBuilding);

        if (!isBuilding) {
            if (startCompletion(rowRef)) return true;
            clearOverlayNodes(rowRef);
            return true;
        }

        if (buildState.status === 'completed') {
            if (completionPlayedByPlugin.has(rowRef.pluginId) && !rowRef.completing) {
                clearOverlayNodes(rowRef);
                return true;
            }
            if (!rowRef.overlay || !rowRef.overlay.isConnected) {
                return true;
            }
            if (rowRef.main) {
                rowRef.main.textContent = 'Completed';
                rowRef.lastMain = 'Completed';
            }
            if (rowRef.sub) {
                rowRef.sub.textContent = 'Reloading plugin';
                rowRef.lastSub = 'Reloading plugin';
            }
            startCompletion(rowRef, true);
            return true;
        }

        if (!ensureOverlayNodes(rowRef)) return false;

        const label = buildState.status === 'queued' ? 'Queued' : 'Compiling';
        const detail = formatDetail(buildState.phase, buildState.percent);
        const normalizedPercent = normalizePercent(buildState.percent);
        resetStaleProgressState(rowRef, buildState.status, normalizedPercent);
        const displayPercent = toDisplayPercent(rowRef, normalizedPercent, buildState.status);
        setFillTarget(rowRef, displayPercent, buildState.status !== 'building');
        if (rowRef.lastMain !== label && rowRef.main) {
            rowRef.main.textContent = label;
            rowRef.lastMain = label;
        }
        if (rowRef.lastSub !== detail && rowRef.sub) {
            rowRef.sub.textContent = detail;
            rowRef.lastSub = detail;
        }

        return true;
    }

    function ensureOverlayNodes(rowRef) {
        if (!rowRef.overlayHost) return false;
        if (rowRef.overlay && rowRef.overlay.isConnected) return true;

        const hadAnimationState = Number.isFinite(rowRef.displayPercent);

        const overlay = document.createElement('div');
        overlay.className = 'plugin-build-overlay progress-track progress-track-direct is-downloading compiling';
        overlay.setAttribute('aria-hidden', 'true');

        const fill = document.createElement('div');
        fill.className = 'progress-fill';
        overlay.appendChild(fill);

        const copy = document.createElement('div');
        copy.className = 'plugin-build-overlay-copy';

        const main = document.createElement('span');
        main.className = 'plugin-build-overlay-main';
        copy.appendChild(main);

        const sub = document.createElement('span');
        sub.className = 'plugin-build-overlay-sub';
        copy.appendChild(sub);

        overlay.appendChild(copy);
        rowRef.overlayHost.replaceChildren(overlay);

        rowRef.overlay = overlay;
        rowRef.fill = fill;
        rowRef.main = main;
        rowRef.sub = sub;

        if (hadAnimationState) {
            applyFillScale(rowRef, rowRef.displayPercent);
            return true;
        }

        rowRef.lastFrameTime = 0;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
        return true;
    }

    function clearOverlayNodes(rowRef) {
        stopFillAnimation(rowRef);
        stopCompletion(rowRef);
        if (rowRef.overlayHost && rowRef.overlayHost.childElementCount > 0) {
            rowRef.overlayHost.replaceChildren();
        }
        rowRef.overlay = null;
        rowRef.fill = null;
        rowRef.main = null;
        rowRef.sub = null;
        rowRef.displayPercent = Number.NaN;
        rowRef.targetPercent = Number.NaN;
        rowRef.lastBuildPercent = Number.NaN;
        rowRef.lastFrameTime = 0;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
    }

    function startCompletion(rowRef, force = false, onDone = null) {
        if (rowRef.completing) {
            if (typeof onDone === 'function') {
                rowRef.completionWaiters.push(onDone);
            }
            return true;
        }
        if (!rowRef.overlay || !rowRef.fill) return false;
        const completedPercent = Math.max(finiteOr(rowRef.displayPercent, 0), finiteOr(rowRef.lastBuildPercent, 0));
        if (!force && completedPercent < BUILD_ANIMATION.completionTriggerPercent) return false;
        if (completionPlayedByPlugin.has(rowRef.pluginId)) return true;
        rowRef.completing = true;
        completionPlayedByPlugin.add(rowRef.pluginId);
        if (typeof onDone === 'function') {
            rowRef.completionWaiters.push(onDone);
        }
        stopFillAnimation(rowRef);
        rowRef.displayPercent = 100;
        rowRef.targetPercent = 100;
        rowRef.lastBuildPercent = 100;
        applyFillScale(rowRef, 100);
        rowRef.overlay.classList.remove('is-completing');
        if (rowRef.main) {
            rowRef.main.textContent = 'Completed';
            rowRef.lastMain = 'Completed';
        }
        if (rowRef.sub) {
            rowRef.sub.textContent = 'Reloading plugin';
            rowRef.lastSub = 'Reloading plugin';
        }
        rowRef.completionFrame = requestAnimationFrame(() => {
            rowRef.completionFrame = null;
            if (!rowRef.overlay) return;
            rowRef.overlay.classList.add('is-completing');
        });

        rowRef.completionTimer = setTimeout(() => {
            const waiters = rowRef.completionWaiters.slice();
            rowRef.completionWaiters = [];
            rowRef.completionTimer = null;
            rowRef.completing = false;
            clearOverlayNodes(rowRef);
            for (const waiter of waiters) {
                waiter();
            }
        }, BUILD_ANIMATION.completionVisibleMs);
        return true;
    }

    function toDisplayPercent(rowRef, normalizedPercent, status) {
        if (status !== 'building') return normalizedPercent;
        if (!Number.isFinite(rowRef.lastBuildPercent)) return normalizedPercent;
        if (normalizedPercent >= rowRef.lastBuildPercent) return normalizedPercent;
        return rowRef.lastBuildPercent;
    }

    function resetStaleProgressState(rowRef, status, normalizedPercent) {
        if (status !== 'queued' && status !== 'building') return;
        if (normalizedPercent > BUILD_ANIMATION.staleResetPercent) return;
        if (!Number.isFinite(rowRef.lastBuildPercent) && !Number.isFinite(rowRef.displayPercent)) return;
        completionPlayedByPlugin.delete(rowRef.pluginId);
        rowRef.displayPercent = Number.NaN;
        rowRef.targetPercent = Number.NaN;
        rowRef.lastBuildPercent = Number.NaN;
        rowRef.lastFrameTime = 0;
        stopFillAnimation(rowRef);
    }

    function setFillTarget(rowRef, targetPercent, immediate) {
        const nextPercent = normalizePercent(targetPercent);
        rowRef.lastBuildPercent = nextPercent;
        if (!rowRef.fill) return;

        if (nextPercent < BUILD_ANIMATION.completionTriggerPercent) {
            completionPlayedByPlugin.delete(rowRef.pluginId);
        }

        if (nextPercent >= BUILD_ANIMATION.completionTriggerPercent) {
            if (completionPlayedByPlugin.has(rowRef.pluginId) && !rowRef.completing) {
                clearOverlayNodes(rowRef);
                return;
            }
            startCompletion(rowRef, true);
            return;
        }

        if (immediate) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent);
            stopFillAnimation(rowRef);
            return;
        }

        if (!Number.isFinite(rowRef.displayPercent)) {
            rowRef.displayPercent = 0;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, 0);
            rowRef.lastFrameTime = performance.now();
            if (rowRef.animationFrame !== null) return;
            rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
            return;
        }

        const delta = Math.abs(nextPercent - rowRef.displayPercent);
        if (delta <= 0.01) {
            rowRef.displayPercent = nextPercent;
            rowRef.targetPercent = nextPercent;
            applyFillScale(rowRef, nextPercent);
            stopFillAnimation(rowRef);
            return;
        }

        rowRef.targetPercent = nextPercent;
        rowRef.lastFrameTime = performance.now();
        if (rowRef.animationFrame !== null) return;
        rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
    }

    function animateFill(rowRef, timestamp) {
        rowRef.animationFrame = null;
        if (!rowRef.fill) return;

        const current = finiteOr(rowRef.displayPercent, 0);
        const target = finiteOr(rowRef.targetPercent, current);
        const delta = target - current;
        if (delta > BUILD_ANIMATION.hardSyncDelta) {
            rowRef.displayPercent = target;
            applyFillScale(rowRef, target);
            rowRef.lastFrameTime = timestamp;
            return;
        }

        if (Math.abs(delta) <= BUILD_ANIMATION.snapDelta) {
            rowRef.displayPercent = target;
            applyFillScale(rowRef, target);
            rowRef.lastFrameTime = timestamp;
            return;
        }

        const elapsed = rowRef.lastFrameTime > 0 ? timestamp - rowRef.lastFrameTime : 16;
        const dt = Math.min(BUILD_ANIMATION.frameMaxMs, Math.max(BUILD_ANIMATION.frameMinMs, elapsed));
        rowRef.lastFrameTime = timestamp;
        const alpha = 1 - Math.exp(-dt / BUILD_ANIMATION.easeMs);
        const eased = current + delta * alpha;
        const next = Math.max(current, eased);

        rowRef.displayPercent = next;
        applyFillScale(rowRef, next);
        rowRef.animationFrame = requestAnimationFrame(ts => animateFill(rowRef, ts));
    }

    function applyFillScale(rowRef, percent) {
        if (!rowRef.fill) return;
        rowRef.fill.style.setProperty('--progress-width', `${normalizePercent(percent).toFixed(2)}%`);
    }

    function stopFillAnimation(rowRef) {
        if (rowRef.animationFrame !== null) {
            cancelAnimationFrame(rowRef.animationFrame);
            rowRef.animationFrame = null;
        }
        rowRef.lastFrameTime = 0;
    }

    function stopCompletion(rowRef) {
        if (rowRef.completionFrame !== null) {
            cancelAnimationFrame(rowRef.completionFrame);
            rowRef.completionFrame = null;
        }
        if (rowRef.completionTimer === null) return;
        const waiters = rowRef.completionWaiters.slice();
        clearTimeout(rowRef.completionTimer);
        rowRef.completionTimer = null;
        rowRef.completing = false;
        rowRef.completionWaiters = [];
        if (waiters.length === 0) return;
        setTimeout(() => {
            for (const waiter of waiters) {
                waiter();
            }
        }, 0);
    }

    function finiteOr(value, fallback) {
        return Number.isFinite(value) ? value : fallback;
    }

    return {
        clearQueued,
        completeRows,
        queue,
        cacheRows,
        syncAll
    };
}

export function createPluginBuildOverlayController({
    getContainer,
    getPluginById,
    getBuildState,
    formatDetail,
    normalizePercent
}) {
    let rowRefs = new Map();
    const pendingBuildRows = new Set();
    let buildSyncFrame = null;

    function clearQueued() {
        pendingBuildRows.clear();
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
    }

    function queue(pluginId, onNeedsFullRender) {
        if (!pluginId) return;
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
        // Cancel pending RAF so it doesn't access stale row refs
        if (buildSyncFrame !== null) {
            cancelAnimationFrame(buildSyncFrame);
            buildSyncFrame = null;
        }
        rowRefs = new Map();
        const container = getContainer();
        if (!container) return;

        const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
        for (const row of rows) {
            const pluginId = row.dataset.pluginId;
            if (!pluginId) continue;
            rowRefs.set(pluginId, {
                row,
                overlayHost: row.querySelector('.plugin-build-overlay-host'),
                overlay: null,
                fill: null,
                main: null,
                sub: null,
                lastScale: -1,
                lastMain: '',
                lastSub: ''
            });
        }
    }

    function syncAll(pluginIds, onNeedsFullRender) {
        let needsFullRender = false;
        for (const pluginId of pluginIds) {
            if (!syncRow(pluginId)) {
                needsFullRender = true;
                break;
            }
        }
        if (needsFullRender && typeof onNeedsFullRender === 'function') {
            onNeedsFullRender();
        }
    }

    function syncRow(pluginId) {
        const container = getContainer();
        if (!container) return false;

        const rowRef = rowRefs.get(pluginId);
        if (!rowRef) return false;

        const plugin = getPluginById(pluginId);
        if (!plugin) return false;

        const buildState = getBuildState(plugin);
        const isBuilding = !!buildState;
        rowRef.row.classList.toggle('is-building', isBuilding);

        if (!isBuilding) {
            clearOverlayNodes(rowRef);
            return true;
        }

        if (!ensureOverlayNodes(rowRef)) {
            return false;
        }

        const label = buildState.status === 'queued' ? 'Queued' : 'Compiling';
        const detail = formatDetail(buildState.phase, buildState.percent);
        const scale = normalizePercent(buildState.percent) / 100;

        if (rowRef.lastScale !== scale && rowRef.fill) {
            rowRef.fill.style.transform = `scaleX(${scale})`;
            rowRef.lastScale = scale;
        }
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

        const overlay = document.createElement('div');
        overlay.className = 'plugin-build-overlay is-downloading compiling';
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
        rowRef.lastScale = -1;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
        return true;
    }

    function clearOverlayNodes(rowRef) {
        if (rowRef.overlayHost && rowRef.overlayHost.childElementCount > 0) {
            rowRef.overlayHost.replaceChildren();
        }
        rowRef.overlay = null;
        rowRef.fill = null;
        rowRef.main = null;
        rowRef.sub = null;
        rowRef.lastScale = -1;
        rowRef.lastMain = '';
        rowRef.lastSub = '';
    }

    return {
        clearQueued,
        queue,
        cacheRows,
        syncAll
    };
}

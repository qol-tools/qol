export function cacheRowRefs(getContainer, previousRows, getCompletionSnapshot) {
    const nextRows = new Map();
    const container = getContainer();
    if (!container) return nextRows;

    const rows = container.querySelectorAll('.plugin-row[data-plugin-id]');
    for (const row of rows) {
        const pluginId = row.dataset.pluginId;
        if (!pluginId) continue;
        const previous = previousRows.get(pluginId);
        nextRows.set(pluginId, makeRowRef(row, previous, getCompletionSnapshot));
    }
    return nextRows;
}

export function ensureOverlayNodes(rowRef, restoreFill) {
    if (!rowRef.overlayHost) return false;
    if (rowRef.overlay && rowRef.overlay.isConnected) return true;

    const hadAnimationState = Number.isFinite(rowRef.displayPercent);
    const overlay = document.createElement('div');
    overlay.className = 'plugin-build-overlay progress-track is-downloading compiling';
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

    if (hadAnimationState && typeof restoreFill === 'function') {
        restoreFill(rowRef.displayPercent);
        return true;
    }

    rowRef.lastFrameTime = 0;
    rowRef.lastMain = '';
    rowRef.lastSub = '';
    return true;
}

export function clearOverlayNodes(rowRef, stopFillAnimation) {
    stopFillAnimation(rowRef);
    if (rowRef.fill) {
        rowRef.fill.style.removeProperty('--progress-transition-override');
    }
    if (rowRef.overlayHost && rowRef.overlayHost.childElementCount > 0) {
        rowRef.overlayHost.replaceChildren();
    }
    rowRef.overlay = null;
    rowRef.fill = null;
    rowRef.main = null;
    rowRef.sub = null;
    rowRef.completing = false;
    rowRef.displayPercent = Number.NaN;
    rowRef.targetPercent = Number.NaN;
    rowRef.lastBuildPercent = Number.NaN;
    rowRef.lastFrameTime = 0;
    rowRef.lastMain = '';
    rowRef.lastSub = '';
}

export function setOverlayCopy(rowRef, mainText, subText) {
    if (rowRef.main && rowRef.lastMain !== mainText) {
        rowRef.main.textContent = mainText;
        rowRef.lastMain = mainText;
    }
    if (rowRef.sub && rowRef.lastSub !== subText) {
        rowRef.sub.textContent = subText;
        rowRef.lastSub = subText;
    }
}

export function applyFillScale(rowRef, percent, normalizePercent) {
    if (!rowRef.fill) return;
    rowRef.fill.style.setProperty('--progress-width', `${normalizePercent(percent).toFixed(2)}%`);
}

export function finiteOr(value, fallback) {
    return Number.isFinite(value) ? value : fallback;
}

function makeRowRef(row, previous, getCompletionSnapshot) {
    const pluginId = row.dataset.pluginId || '';
    const snapshot = getCompletionSnapshot(pluginId, performance.now());
    const preserveOverlay = previous?.overlay?.isConnected;
    return {
        row,
        pluginId,
        overlayHost: row.querySelector('.plugin-build-overlay-host'),
        overlay: preserveOverlay ? previous.overlay : null,
        fill: preserveOverlay ? previous.fill : null,
        main: preserveOverlay ? previous.main : null,
        sub: preserveOverlay ? previous.sub : null,
        displayPercent: finiteOr(previous?.displayPercent, finiteOr(snapshot?.percent, Number.NaN)),
        targetPercent: finiteOr(previous?.targetPercent, finiteOr(snapshot?.percent, Number.NaN)),
        lastBuildPercent: finiteOr(previous?.lastBuildPercent, finiteOr(snapshot?.percent, Number.NaN)),
        animationFrame: null,
        completing: previous?.completing === true || snapshot?.completing === true,
        lastFrameTime: 0,
        lastMain: '',
        lastSub: ''
    };
}

export function clampPercent(value) {
    if (!Number.isFinite(value)) return 0;
    return Math.max(0, Math.min(100, value));
}

export function normalizePercent(value, { round = false } = {}) {
    const numeric = Number(value);
    const percent = clampPercent(numeric);
    return round ? Math.round(percent) : percent;
}

export function toProgressScale(percent) {
    return clampPercent(percent) / 100;
}

export function formatDownloadingProgress(percent) {
    const normalized = normalizePercent(percent, { round: true });
    return normalized > 0 ? `Downloading ${normalized}%` : 'Downloading...';
}

export function formatPhaseProgress(phase, percent, fallbackPhase) {
    const normalized = normalizePercent(percent, { round: true });
    const resolvedPhase = typeof phase === 'string' && phase.trim()
        ? phase
        : fallbackPhase;
    return normalized > 0 ? `${resolvedPhase} ${normalized}%` : `${resolvedPhase}...`;
}

export function formatBuildOverlayDetail(phase, percent) {
    const normalized = normalizePercent(percent, { round: true });
    const resolvedPhase = typeof phase === 'string' ? phase.trim() : '';
    return resolvedPhase ? `${resolvedPhase} • ${normalized}%` : `${normalized}%`;
}

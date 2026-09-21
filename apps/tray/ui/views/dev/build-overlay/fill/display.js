export function toDisplayPercent(rowRef, normalizedPercent, status) {
    if (status !== 'building') {
        return normalizedPercent;
    }
    if (!Number.isFinite(rowRef.lastBuildPercent)) {
        return normalizedPercent;
    }
    if (normalizedPercent >= rowRef.lastBuildPercent) {
        return normalizedPercent;
    }
    return rowRef.lastBuildPercent;
}

const PRESETS = new Set(['wood', 'parchment', 'terminal', 'spacecraft']);

export function isKnownPreset(name) {
    return typeof name === 'string' && PRESETS.has(name);
}

export function resolvePresetClass(atmosphere) {
    if (!atmosphere || typeof atmosphere !== 'object') return null;
    const { preset } = atmosphere;
    if (!isKnownPreset(preset)) return null;
    return `atmosphere-preset-${preset}`;
}

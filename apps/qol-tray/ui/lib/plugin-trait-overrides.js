const OVERRIDES = {
    'plugin-lights': {
        confined: {},
        'peripheral-preview': { neighbors: 1 },
        atmosphere: { preset: 'wood' },
    },
    'plugin-alt-tab': {
        confined: {},
        'peripheral-preview': { neighbors: 1 },
        atmosphere: { preset: 'spacecraft' },
    },
    'plugin-pointz': {
        confined: {},
        'peripheral-preview': { neighbors: 1 },
        atmosphere: { preset: 'terminal' },
    },
};

export function pluginTraitOverride(pluginId) {
    return OVERRIDES[pluginId] || null;
}

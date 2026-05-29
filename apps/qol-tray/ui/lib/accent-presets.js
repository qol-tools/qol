export const ACCENT_PRESETS = {
    amber: { label: 'Amber', rgb: '255, 180, 84', hover: '#ffc77a' },
    green: { label: 'Green', rgb: '70, 224, 138', hover: '#7ff0ab' },
    cyan: { label: 'Cyan', rgb: '86, 214, 224', hover: '#8fe8f0' },
    magenta: { label: 'Magenta', rgb: '232, 121, 198', hover: '#f49ad6' },
    blue: { label: 'Blue', rgb: '74, 158, 255', hover: '#68b0ff' },
};

export const DEFAULT_ACCENT = 'amber';
export const DEV_ACCENT = 'green';

export function resolveAccent(setting, devEnabled) {
    if (setting && ACCENT_PRESETS[setting]) return setting;
    return devEnabled ? DEV_ACCENT : DEFAULT_ACCENT;
}

export function applyAccent(presetKey) {
    const preset = ACCENT_PRESETS[presetKey] || ACCENT_PRESETS[DEFAULT_ACCENT];
    const root = document.documentElement.style;
    root.setProperty('--accent-rgb', preset.rgb);
    root.setProperty('--accent-hover', preset.hover);
}

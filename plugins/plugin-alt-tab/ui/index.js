const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    display: {
        preview_mode: 'below_list',
        max_columns: 6,
        preview_fps: 10,
        transparent_background: false,
        card_background_color: '1a1e2a',
        card_background_opacity: 0.85,
        show_hotkey_hints: true
    },
    action_mode: 'sticky',
    reset_selection_on_open: true,
    label: {
        show_app_name: true,
        show_window_title: true
    }
};

const PREVIEW_MODES = new Set(['below_list', 'preview_only']);
const ACTION_MODES = new Set(['sticky', 'hold_to_switch']);

const elements = {
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status'),
    layoutMock: document.getElementById('layout-mock')
};

function selectedModeInput() {
    return document.querySelector('input[name="preview-mode"]:checked');
}

function selectedActionModeInput() {
    return document.querySelector('input[name="action-mode"]:checked');
}

function normalizeConfig(raw) {
    const previewMode = raw?.display?.preview_mode;
    const maxColumns = parseInt(raw?.display?.max_columns, 10) || DEFAULT_CONFIG.display.max_columns;
    const previewFps = parseInt(raw?.display?.preview_fps, 10);
    const normalizedFps = isNaN(previewFps) ? DEFAULT_CONFIG.display.preview_fps : Math.max(0, Math.min(60, previewFps));
    const transparentBackground = typeof raw?.display?.transparent_background === 'boolean'
        ? raw.display.transparent_background
        : DEFAULT_CONFIG.display.transparent_background;
    const cardBgColor = (typeof raw?.display?.card_background_color === 'string' && /^[0-9a-fA-F]{6}$/.test(raw.display.card_background_color))
        ? raw.display.card_background_color
        : DEFAULT_CONFIG.display.card_background_color;
    const cardBgOpacity = typeof raw?.display?.card_background_opacity === 'number'
        ? Math.max(0, Math.min(1, raw.display.card_background_opacity))
        : DEFAULT_CONFIG.display.card_background_opacity;
    const actionMode = raw?.action_mode;
    const resetSelectionOnOpen = typeof raw?.reset_selection_on_open === 'boolean'
        ? raw.reset_selection_on_open
        : DEFAULT_CONFIG.reset_selection_on_open;
    
    const showHotkeyHints = typeof raw?.display?.show_hotkey_hints === 'boolean'
        ? raw.display.show_hotkey_hints
        : DEFAULT_CONFIG.display.show_hotkey_hints;

    return {
        display: {
            preview_mode: PREVIEW_MODES.has(previewMode) ? previewMode : DEFAULT_CONFIG.display.preview_mode,
            max_columns: Math.max(2, Math.min(12, maxColumns)),
            preview_fps: normalizedFps,
            transparent_background: transparentBackground,
            card_background_color: cardBgColor,
            card_background_opacity: cardBgOpacity,
            show_hotkey_hints: showHotkeyHints
        },
        action_mode: ACTION_MODES.has(actionMode) ? actionMode : DEFAULT_CONFIG.action_mode,
        reset_selection_on_open: resetSelectionOnOpen,
        label: {
            show_app_name: typeof raw?.label?.show_app_name === 'boolean' ? raw.label.show_app_name : DEFAULT_CONFIG.label.show_app_name,
            show_window_title: typeof raw?.label?.show_window_title === 'boolean' ? raw.label.show_window_title : DEFAULT_CONFIG.label.show_window_title,
        }
    };
}

function applyConfigToUI(config) {
    const previewMode = config.display.preview_mode;
    const previewInput = document.querySelector(`input[name="preview-mode"][value="${previewMode}"]`);
    if (previewInput) {
        previewInput.checked = true;
    }
    elements.layoutMock.dataset.mode = previewMode;

    const maxColumns = config.display.max_columns;
    const maxColumnsInput = document.getElementById('max-columns');
    if (maxColumnsInput) {
        maxColumnsInput.value = maxColumns;
    }
    updateGridVisualizer();

    const previewFps = config.display.preview_fps;
    const previewFpsInput = document.getElementById('preview-fps');
    if (previewFpsInput) {
        previewFpsInput.value = previewFps;
        document.getElementById('preview-fps-value').textContent = previewFps;
    }

    const showHotkeyHintsInput = document.getElementById('show-hotkey-hints');
    if (showHotkeyHintsInput) {
        showHotkeyHintsInput.checked = config.display.show_hotkey_hints !== false;
    }

    const transparentBgInput = document.getElementById('transparent-background');
    if (transparentBgInput) {
        transparentBgInput.checked = !!config.display.transparent_background;
    }

    const cardBgColorInput = document.getElementById('card-bg-color');
    if (cardBgColorInput) {
        cardBgColorInput.value = '#' + (config.display.card_background_color || '1a1e2a');
    }
    const cardBgOpacityInput = document.getElementById('card-bg-opacity');
    if (cardBgOpacityInput) {
        const pct = Math.round((config.display.card_background_opacity ?? 0.85) * 100);
        cardBgOpacityInput.value = pct;
        document.getElementById('card-bg-opacity-value').textContent = pct + '%';
    }
    updateCardBgVisibility();

    const actionMode = config.action_mode;
    const actionInput = document.querySelector(`input[name="action-mode"][value="${actionMode}"]`);
    if (actionInput) {
        actionInput.checked = true;
    }

    const resetSelectionOnOpenInput = document.getElementById('reset-selection-on-open');
    if (resetSelectionOnOpenInput) {
        resetSelectionOnOpenInput.checked = !!config.reset_selection_on_open;
    }

    const showAppNameInput = document.getElementById('show-app-name');
    if (showAppNameInput) {
        showAppNameInput.checked = !!config.label.show_app_name;
    }
    const showWindowTitleInput = document.getElementById('show-window-title');
    if (showWindowTitleInput) {
        showWindowTitleInput.checked = !!config.label.show_window_title;
    }
    updateLabelVisualizer();
}

function collectConfigFromUI() {
    const previewSelected = selectedModeInput()?.value;
    const maxColumnsSelected = parseInt(document.getElementById('max-columns')?.value, 10);
    const actionSelected = selectedActionModeInput()?.value;
    const resetSelectionOnOpenSelected = document.getElementById('reset-selection-on-open')?.checked;
    const showHotkeyHintsSelected = document.getElementById('show-hotkey-hints')?.checked;
    const transparentBgSelected = document.getElementById('transparent-background')?.checked;
    const cardBgColorRaw = (document.getElementById('card-bg-color')?.value || '#1a1e2a').replace('#', '');
    const cardBgOpacityRaw = parseInt(document.getElementById('card-bg-opacity')?.value, 10) / 100;
    const showAppNameSelected = document.getElementById('show-app-name')?.checked;
    const showWindowTitleSelected = document.getElementById('show-window-title')?.checked;

    return normalizeConfig({
        display: {
            preview_mode: previewSelected,
            max_columns: maxColumnsSelected || 6,
            preview_fps: parseInt(document.getElementById('preview-fps')?.value, 10) || 10,
            transparent_background: !!transparentBgSelected,
            card_background_color: cardBgColorRaw,
            card_background_opacity: isNaN(cardBgOpacityRaw) ? 0.85 : cardBgOpacityRaw,
            show_hotkey_hints: !!showHotkeyHintsSelected
        },
        action_mode: actionSelected,
        reset_selection_on_open: resetSelectionOnOpenSelected,
        label: {
            show_app_name: showAppNameSelected,
            show_window_title: showWindowTitleSelected
        }
    });
}

function setStatus(text, isError = false) {
    elements.saveStatus.textContent = text;
    elements.saveStatus.classList.toggle('error', isError);
}

async function loadConfig() {
    let config = { ...DEFAULT_CONFIG };

    try {
        const response = await fetch(CONFIG_URL);
        if (response.ok) {
            config = normalizeConfig(await response.json());
        }
    } catch (error) {
        console.warn('Could not load alt-tab config, using defaults', error);
    }

    applyConfigToUI(config);
}

async function saveConfig() {
    const nextConfig = collectConfigFromUI();

    elements.saveBtn.disabled = true;
    setStatus('Saving...');

    try {
        const response = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(nextConfig, null, 2)
        });

        if (!response.ok) {
            throw new Error(`Save failed with status ${response.status}`);
        }

        setStatus('Saved');
        setTimeout(() => setStatus(''), 2000);
    } catch (error) {
        console.error('Failed to save alt-tab config', error);
        setStatus('Failed to save', true);
        setTimeout(() => setStatus(''), 3000);
    } finally {
        elements.saveBtn.disabled = false;
    }
}

document.querySelectorAll('input[name="preview-mode"]').forEach((input) => {
    input.addEventListener('change', () => {
        elements.layoutMock.dataset.mode = input.value;
    });
});

function updateGridVisualizer() {
    const maxColsInput = document.getElementById('max-columns');
    const simWinsInput = document.getElementById('sim-windows');

    if (!maxColsInput || !simWinsInput) return;

    const maxCols = parseInt(maxColsInput.value, 10);
    const simWins = parseInt(simWinsInput.value, 10);

    document.getElementById('max-columns-value').textContent = maxCols;
    document.getElementById('sim-windows-value').textContent = simWins;

    const count = Math.max(1, simWins);
    let cols = 1;
    if (count > 1) {
        cols = Math.min(count, Math.max(2, maxCols));
    }

    const visualizer = document.getElementById('grid-visualizer');
    visualizer.style.gridTemplateColumns = `repeat(${cols}, 1fr)`;

    visualizer.innerHTML = '';
    for (let i = 0; i < count; i++) {
        const item = document.createElement('div');
        item.className = 'grid-vis-item';
        visualizer.appendChild(item);
    }
}

document.getElementById('max-columns')?.addEventListener('input', updateGridVisualizer);
document.getElementById('sim-windows')?.addEventListener('input', updateGridVisualizer);

document.getElementById('preview-fps')?.addEventListener('input', () => {
    document.getElementById('preview-fps-value').textContent =
        document.getElementById('preview-fps').value;
});

function updateLabelVisualizer() {
    const showApp = document.getElementById('show-app-name')?.checked;
    const showTitle = document.getElementById('show-window-title')?.checked;
    const visualizer = document.getElementById('label-visualizer-text');
    
    if (!visualizer) return;

    if (showApp && showTitle) {
        visualizer.textContent = 'Firefox - Search - Google';
    } else if (showApp) {
        visualizer.textContent = 'Firefox';
    } else if (showTitle) {
        visualizer.textContent = 'Search - Google';
    } else {
        visualizer.textContent = '';
    }
}

document.getElementById('show-app-name')?.addEventListener('change', updateLabelVisualizer);
document.getElementById('show-window-title')?.addEventListener('change', updateLabelVisualizer);

function updateCardBgVisibility() {
    const group = document.getElementById('card-bg-group');
    const checked = document.getElementById('transparent-background')?.checked;
    if (group) group.style.display = checked ? '' : 'none';
}

document.getElementById('transparent-background')?.addEventListener('change', updateCardBgVisibility);

document.getElementById('card-bg-opacity')?.addEventListener('input', () => {
    document.getElementById('card-bg-opacity-value').textContent =
        document.getElementById('card-bg-opacity').value + '%';
});

elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (event) => {
    if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        saveConfig();
    }
});

loadConfig();

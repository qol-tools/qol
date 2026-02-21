const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    display: {
        preview_mode: 'below_list',
        max_columns: 6
    },
    action_mode: 'sticky'
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
    const actionMode = raw?.action_mode;
    return {
        display: {
            preview_mode: PREVIEW_MODES.has(previewMode) ? previewMode : DEFAULT_CONFIG.display.preview_mode,
            max_columns: Math.max(2, Math.min(12, maxColumns))
        },
        action_mode: ACTION_MODES.has(actionMode) ? actionMode : DEFAULT_CONFIG.action_mode
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

    const actionMode = config.action_mode;
    const actionInput = document.querySelector(`input[name="action-mode"][value="${actionMode}"]`);
    if (actionInput) {
        actionInput.checked = true;
    }
}

function collectConfigFromUI() {
    const previewSelected = selectedModeInput()?.value;
    const maxColumnsSelected = parseInt(document.getElementById('max-columns')?.value, 10);
    const actionSelected = selectedActionModeInput()?.value;
    return normalizeConfig({
        display: {
            preview_mode: previewSelected,
            max_columns: maxColumnsSelected || 6
        },
        action_mode: actionSelected
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
    
    // Update labels
    document.getElementById('max-columns-value').textContent = maxCols;
    document.getElementById('sim-windows-value').textContent = simWins;
    
    // Logic matching preferred_column_count
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

elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (event) => {
    if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        saveConfig();
    }
});

loadConfig();

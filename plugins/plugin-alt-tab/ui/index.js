const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    display: {
        preview_mode: 'below_list'
    }
};

const MODES = new Set(['below_list', 'preview_only']);

const elements = {
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status'),
    layoutMock: document.getElementById('layout-mock')
};

function selectedModeInput() {
    return document.querySelector('input[name="preview-mode"]:checked');
}

function normalizeConfig(raw) {
    const mode = raw?.display?.preview_mode;
    return {
        display: {
            preview_mode: MODES.has(mode) ? mode : DEFAULT_CONFIG.display.preview_mode
        }
    };
}

function applyConfigToUI(config) {
    const mode = config.display.preview_mode;
    const input = document.querySelector(`input[name="preview-mode"][value="${mode}"]`);
    if (input) {
        input.checked = true;
    }
    elements.layoutMock.dataset.mode = mode;
}

function collectConfigFromUI() {
    const selected = selectedModeInput()?.value;
    return normalizeConfig({
        display: {
            preview_mode: selected
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

elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (event) => {
    if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        saveConfig();
    }
});

loadConfig();

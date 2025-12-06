const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULTS = {
    half_life_days: 7,
    frequency_bonus: 500,
    prefer_apps: true,
    penalize_hidden: true,
    depth_penalty: 2,
    exact_bonus: 0,
    prefix_penalty: 100,
    contains_penalty: 200
};

const elements = {
    halfLife: document.getElementById('half-life'),
    frequencyBonus: document.getElementById('frequency-bonus'),
    preferApps: document.getElementById('prefer-apps'),
    penalizeHidden: document.getElementById('penalize-hidden'),
    depthPenalty: document.getElementById('depth-penalty'),
    exactBonus: document.getElementById('exact-bonus'),
    prefixPenalty: document.getElementById('prefix-penalty'),
    containsPenalty: document.getElementById('contains-penalty'),
    saveBtn: document.getElementById('save-btn'),
    resetBtn: document.getElementById('reset-btn'),
    saveStatus: document.getElementById('save-status')
};

let config = { ...DEFAULTS };

async function loadConfig() {
    try {
        const response = await fetch(CONFIG_URL);
        if (response.ok) {
            const loaded = await response.json();
            config = { ...DEFAULTS, ...loaded };
        }
    } catch (e) {
        console.warn('Could not load config, using defaults');
    }
    applyConfigToUI();
}

function applyConfigToUI() {
    elements.halfLife.value = config.half_life_days;
    elements.frequencyBonus.value = config.frequency_bonus;
    elements.preferApps.checked = config.prefer_apps;
    elements.penalizeHidden.checked = config.penalize_hidden;
    elements.depthPenalty.value = config.depth_penalty;
    elements.exactBonus.value = config.exact_bonus;
    elements.prefixPenalty.value = config.prefix_penalty;
    elements.containsPenalty.value = config.contains_penalty;
}

function collectConfigFromUI() {
    return {
        half_life_days: parseInt(elements.halfLife.value, 10) || DEFAULTS.half_life_days,
        frequency_bonus: parseInt(elements.frequencyBonus.value, 10) || 0,
        prefer_apps: elements.preferApps.checked,
        penalize_hidden: elements.penalizeHidden.checked,
        depth_penalty: parseInt(elements.depthPenalty.value, 10) || 0,
        exact_bonus: parseInt(elements.exactBonus.value, 10) || 0,
        prefix_penalty: parseInt(elements.prefixPenalty.value, 10) || 0,
        contains_penalty: parseInt(elements.containsPenalty.value, 10) || 0
    };
}

async function saveConfig() {
    const newConfig = collectConfigFromUI();

    elements.saveBtn.disabled = true;
    elements.saveStatus.textContent = 'Saving...';

    try {
        const response = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(newConfig, null, 2)
        });

        if (!response.ok) throw new Error('Save failed');

        config = newConfig;
        elements.saveStatus.textContent = 'Saved';
        setTimeout(() => { elements.saveStatus.textContent = ''; }, 2000);
    } catch (e) {
        elements.saveStatus.textContent = 'Failed to save';
        elements.saveStatus.style.color = '#ff6b6b';
        setTimeout(() => {
            elements.saveStatus.textContent = '';
            elements.saveStatus.style.color = '';
        }, 3000);
    } finally {
        elements.saveBtn.disabled = false;
    }
}

function resetToDefaults() {
    config = { ...DEFAULTS };
    applyConfigToUI();
}

elements.saveBtn.addEventListener('click', saveConfig);
elements.resetBtn.addEventListener('click', resetToDefaults);

document.addEventListener('keydown', (e) => {
    if (e.key === 's' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        saveConfig();
    }
});

loadConfig();

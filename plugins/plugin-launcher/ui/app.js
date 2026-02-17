const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const DEFAULT_CONFIG = {
    monitor: {
        poll_min_ms: 16,
        poll_max_ms: 500,
        commit_threshold_ms: 128,
        strategy: 'basic'
    }
};

const STRATEGIES = {
    basic: {
        label: 'Basic',
        description: 'Basic uses fixed half/double steps.'
    },
    momentum: {
        label: 'Momentum',
        description: 'Momentum adapts ramp speed based on streaks.'
    }
};

const LIMITS = {
    pollMin: { min: 8, max: 100 },
    pollMax: { min: 200, max: 2000 },
    commitThreshold: { min: 50, max: 500 }
};

const elements = {
    pollMinMs: document.getElementById('poll-min-ms'),
    pollMaxMs: document.getElementById('poll-max-ms'),
    commitThresholdMs: document.getElementById('commit-threshold-ms'),
    commitThresholdValue: document.getElementById('commit-threshold-value'),
    strategy: document.getElementById('strategy'),
    strategyHint: document.getElementById('strategy-hint'),
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status')
};

let config = {
    monitor: { ...DEFAULT_CONFIG.monitor }
};

function clampNumber(raw, min, max, fallback) {
    const value = Number.parseInt(raw, 10);
    if (!Number.isFinite(value)) {
        return fallback;
    }
    return Math.min(max, Math.max(min, value));
}

function normalizeConfig(rawConfig) {
    const rawMonitor = rawConfig?.monitor ?? {};
    const strategy = Object.prototype.hasOwnProperty.call(STRATEGIES, rawMonitor.strategy)
        ? rawMonitor.strategy
        : DEFAULT_CONFIG.monitor.strategy;

    return {
        monitor: {
            poll_min_ms: clampNumber(
                rawMonitor.poll_min_ms,
                LIMITS.pollMin.min,
                LIMITS.pollMin.max,
                DEFAULT_CONFIG.monitor.poll_min_ms
            ),
            poll_max_ms: clampNumber(
                rawMonitor.poll_max_ms,
                LIMITS.pollMax.min,
                LIMITS.pollMax.max,
                DEFAULT_CONFIG.monitor.poll_max_ms
            ),
            commit_threshold_ms: clampNumber(
                rawMonitor.commit_threshold_ms,
                LIMITS.commitThreshold.min,
                LIMITS.commitThreshold.max,
                DEFAULT_CONFIG.monitor.commit_threshold_ms
            ),
            strategy
        }
    };
}

function renderStrategyOptions() {
    const fragment = document.createDocumentFragment();
    for (const [value, strategy] of Object.entries(STRATEGIES)) {
        const option = document.createElement('option');
        option.value = value;
        option.textContent = strategy.label;
        fragment.appendChild(option);
    }
    elements.strategy.replaceChildren(fragment);
}

function updateStrategyHint() {
    const fallback = STRATEGIES[DEFAULT_CONFIG.monitor.strategy];
    const selected = STRATEGIES[elements.strategy.value] ?? fallback;
    elements.strategyHint.textContent = selected.description;
}

function applyConfigToUI() {
    elements.pollMinMs.value = String(config.monitor.poll_min_ms);
    elements.pollMaxMs.value = String(config.monitor.poll_max_ms);
    elements.commitThresholdMs.value = String(config.monitor.commit_threshold_ms);
    elements.commitThresholdValue.textContent = String(config.monitor.commit_threshold_ms);
    elements.strategy.value = config.monitor.strategy;
    updateStrategyHint();
}

function collectConfigFromUI() {
    return normalizeConfig({
        monitor: {
            poll_min_ms: elements.pollMinMs.value,
            poll_max_ms: elements.pollMaxMs.value,
            commit_threshold_ms: elements.commitThresholdMs.value,
            strategy: elements.strategy.value
        }
    });
}

async function loadConfig() {
    try {
        const response = await fetch(CONFIG_URL);
        if (response.ok) {
            const loaded = await response.json();
            config = normalizeConfig(loaded);
        }
    } catch (error) {
        console.warn('Could not load launcher config, using defaults', error);
    }

    applyConfigToUI();
}

async function saveConfig() {
    const nextConfig = collectConfigFromUI();

    elements.saveBtn.disabled = true;
    elements.saveStatus.style.color = '';
    elements.saveStatus.textContent = 'Saving...';

    try {
        const response = await fetch(CONFIG_URL, {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(nextConfig, null, 2)
        });

        if (!response.ok) {
            throw new Error(`Save failed with status ${response.status}`);
        }

        config = nextConfig;
        applyConfigToUI();
        elements.saveStatus.textContent = 'Saved';
        setTimeout(() => {
            elements.saveStatus.textContent = '';
        }, 2000);
    } catch (error) {
        elements.saveStatus.textContent = 'Failed to save';
        elements.saveStatus.style.color = '#ff6b6b';
        setTimeout(() => {
            elements.saveStatus.textContent = '';
            elements.saveStatus.style.color = '';
        }, 3000);
        console.error('Failed to save launcher config', error);
    } finally {
        elements.saveBtn.disabled = false;
    }
}

elements.commitThresholdMs.addEventListener('input', () => {
    elements.commitThresholdValue.textContent = elements.commitThresholdMs.value;
});

elements.strategy.addEventListener('change', updateStrategyHint);

elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (event) => {
    if (event.key === 's' && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        saveConfig();
    }
});

renderStrategyOptions();
loadConfig();

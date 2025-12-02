const PLUGIN_ID = window.location.pathname.split('/')[2];
const CONFIG_URL = `/api/plugins/${PLUGIN_ID}/config`;

const elements = {
    audioEnabled: document.getElementById('audio-enabled'),
    inputMic: document.getElementById('input-mic'),
    inputSystem: document.getElementById('input-system'),
    micDevice: document.getElementById('mic-device'),
    systemDevice: document.getElementById('system-device'),
    framerate: document.getElementById('framerate'),
    crf: document.getElementById('crf'),
    crfValue: document.getElementById('crf-value'),
    preset: document.getElementById('preset'),
    format: document.getElementById('format'),
    saveBtn: document.getElementById('save-btn'),
    saveStatus: document.getElementById('save-status')
};

let config = {
    audio: {
        enabled: true,
        inputs: ['mic', 'system'],
        mic_device: 'default',
        system_device: 'default'
    },
    video: {
        crf: 18,
        preset: 'veryfast',
        framerate: 60,
        format: 'mkv'
    }
};

async function loadConfig() {
    try {
        const response = await fetch(CONFIG_URL);
        if (response.ok) {
            config = await response.json();
        }
    } catch (e) {
        console.warn('Could not load config, using defaults');
    }
    applyConfigToUI();
}

function applyConfigToUI() {
    elements.audioEnabled.checked = config.audio?.enabled ?? true;
    
    const inputs = config.audio?.inputs ?? ['mic', 'system'];
    elements.inputMic.checked = inputs.includes('mic');
    elements.inputSystem.checked = inputs.includes('system');
    
    elements.micDevice.value = config.audio?.mic_device ?? 'default';
    elements.systemDevice.value = config.audio?.system_device ?? 'default';
    
    elements.framerate.value = String(config.video?.framerate ?? 60);
    elements.crf.value = config.video?.crf ?? 18;
    elements.crfValue.textContent = elements.crf.value;
    elements.preset.value = config.video?.preset ?? 'veryfast';
    elements.format.value = config.video?.format ?? 'mkv';
    
    updateAudioInputsState();
}

function updateAudioInputsState() {
    const enabled = elements.audioEnabled.checked;
    document.querySelectorAll('.audio-inputs').forEach(el => {
        el.classList.toggle('disabled', !enabled);
    });
}

function collectConfigFromUI() {
    const inputs = [];
    if (elements.inputMic.checked) inputs.push('mic');
    if (elements.inputSystem.checked) inputs.push('system');
    
    return {
        audio: {
            enabled: elements.audioEnabled.checked,
            inputs,
            mic_device: elements.micDevice.value || 'default',
            system_device: elements.systemDevice.value || 'default'
        },
        video: {
            crf: parseInt(elements.crf.value, 10),
            preset: elements.preset.value,
            framerate: parseInt(elements.framerate.value, 10),
            format: elements.format.value
        }
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
        elements.saveStatus.textContent = '✓ Saved';
        setTimeout(() => { elements.saveStatus.textContent = ''; }, 2000);
    } catch (e) {
        elements.saveStatus.textContent = '✗ Failed to save';
        elements.saveStatus.style.color = '#ff6b6b';
        setTimeout(() => { 
            elements.saveStatus.textContent = '';
            elements.saveStatus.style.color = '';
        }, 3000);
    } finally {
        elements.saveBtn.disabled = false;
    }
}

elements.audioEnabled.addEventListener('change', updateAudioInputsState);
elements.crf.addEventListener('input', () => {
    elements.crfValue.textContent = elements.crf.value;
});
elements.saveBtn.addEventListener('click', saveConfig);

document.addEventListener('keydown', (e) => {
    if (e.key === 's' && (e.ctrlKey || e.metaKey)) {
        e.preventDefault();
        saveConfig();
    }
});

loadConfig();


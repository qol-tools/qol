export const id = 'hotkeys';

const state = {
    hotkeys: [],
    plugins: [],
    selectedIndex: -1,
    editModalOpen: false,
    recordingKey: false,
    editingHotkey: null
};

let container = null;

export function render(containerEl) {
    container = containerEl;
    container.innerHTML = `
        <div class="view-container">
            <header>
                <h1>Hotkeys</h1>
                <p>Configure global keyboard shortcuts for plugin actions</p>
            </header>
            <div id="hotkeys-list" class="hotkeys-list"></div>
            <footer class="help">
                ↑↓ navigate • Enter edit • a add • d delete • Space toggle
            </footer>
        </div>
    `;
    
    loadData();
}

async function loadData() {
    const listEl = document.getElementById('hotkeys-list');
    if (!listEl) return;
    
    listEl.addEventListener('click', handleClick);
    
    try {
        const [hotkeysRes, pluginsRes] = await Promise.all([
            fetch('/api/hotkeys'),
            fetch('/api/installed')
        ]);
        
        if (hotkeysRes.ok) {
            const config = await hotkeysRes.json();
            state.hotkeys = config.hotkeys || [];
        }
        
        if (pluginsRes.ok) {
            state.plugins = await pluginsRes.json();
        }
        
        renderList();
        if (state.hotkeys.length > 0) {
            state.selectedIndex = 0;
            updateSelection();
        }
    } catch (error) {
        listEl.innerHTML = `<div class="error">Error loading hotkeys: ${error.message}</div>`;
    }
}

function renderList() {
    const listEl = document.getElementById('hotkeys-list');
    if (!listEl) return;
    
    if (state.hotkeys.length === 0) {
        listEl.innerHTML = `
            <div class="empty">
                No hotkeys configured. Press <kbd>a</kbd> to add one.
            </div>
        `;
        return;
    }
    
    listEl.innerHTML = `
        <div class="hotkey-header">
            <span class="col-key">Shortcut</span>
            <span class="col-plugin">Plugin</span>
            <span class="col-action">Action</span>
            <span class="col-status">Status</span>
        </div>
        ${state.hotkeys.map((hk, index) => {
            const plugin = state.plugins.find(p => p.id === hk.plugin_id);
            const pluginName = plugin?.name || hk.plugin_id;
            
            return `
                <div class="hotkey-row ${hk.enabled ? '' : 'disabled'}" data-index="${index}">
                    <span class="col-key"><kbd>${hk.key}</kbd></span>
                    <span class="col-plugin">${pluginName}</span>
                    <span class="col-action">${hk.action}</span>
                    <span class="col-status">${hk.enabled ? '●' : '○'}</span>
                </div>
            `;
        }).join('')}
    `;
}

function updateSelection() {
    document.querySelectorAll('.hotkey-row').forEach((row, i) => {
        row.classList.toggle('selected', i === state.selectedIndex);
    });
    
    const selected = document.querySelector('.hotkey-row.selected');
    if (selected) {
        selected.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
}

function handleClick(e) {
    if (state.editModalOpen) {
        handleModalClick(e);
        return;
    }
    
    const row = e.target.closest('.hotkey-row');
    if (!row) return;
    
    const index = parseInt(row.dataset.index, 10);
    if (index !== state.selectedIndex) {
        state.selectedIndex = index;
        updateSelection();
    } else {
        openEditModal(state.hotkeys[index]);
    }
}

function handleModalClick(e) {
    if (e.target.classList.contains('edit-modal')) {
        closeEditModal();
        return;
    }
    
    if (e.target.closest('.modal-cancel')) {
        closeEditModal();
        return;
    }
    
    if (e.target.closest('.modal-save')) {
        saveHotkey();
        return;
    }
    
    if (e.target.closest('.key-record-btn')) {
        startKeyRecording();
        return;
    }
}

function openEditModal(hotkey = null) {
    state.editModalOpen = true;
    state.editingHotkey = hotkey;
    state.recordingKey = false;
    
    const isNew = !hotkey;
    const title = isNew ? 'Add Hotkey' : 'Edit Hotkey';
    
    const pluginOptions = state.plugins.map(p => 
        `<option value="${p.id}" ${hotkey?.plugin_id === p.id ? 'selected' : ''}>${p.name}</option>`
    ).join('');
    
    const modal = document.createElement('div');
    modal.className = 'edit-modal';
    modal.innerHTML = `
        <div class="edit-modal-content">
            <h3>${title}</h3>
            
            <div class="form-group">
                <label>Shortcut</label>
                <div class="key-input-row">
                    <input type="text" id="hotkey-key" value="${hotkey?.key || ''}" readonly placeholder="Click Record...">
                    <button class="key-record-btn">Record</button>
                </div>
            </div>
            
            <div class="form-group">
                <label>Plugin</label>
                <select id="hotkey-plugin">
                    <option value="">Select plugin...</option>
                    ${pluginOptions}
                </select>
            </div>
            
            <div class="form-group">
                <label>Action</label>
                <select id="hotkey-action">
                    <option value="run" ${hotkey?.action === 'run' ? 'selected' : ''}>Run</option>
                </select>
            </div>
            
            <div class="form-group">
                <label class="checkbox-label">
                    <input type="checkbox" id="hotkey-enabled" ${hotkey?.enabled !== false ? 'checked' : ''}>
                    Enabled
                </label>
            </div>
            
            <div class="modal-buttons">
                <button class="modal-cancel">Cancel (Esc)</button>
                <button class="modal-save">Save (Enter)</button>
            </div>
        </div>
    `;
    
    container.appendChild(modal);
}

function closeEditModal() {
    const modal = container.querySelector('.edit-modal');
    if (modal) modal.remove();
    state.editModalOpen = false;
    state.editingHotkey = null;
    state.recordingKey = false;
}

function startKeyRecording() {
    state.recordingKey = true;
    const input = document.getElementById('hotkey-key');
    const btn = container.querySelector('.key-record-btn');
    if (input) input.placeholder = 'Press keys...';
    if (btn) btn.textContent = 'Recording...';
}

function stopKeyRecording(key) {
    state.recordingKey = false;
    const input = document.getElementById('hotkey-key');
    const btn = container.querySelector('.key-record-btn');
    if (input) {
        input.value = key;
        input.placeholder = 'Click Record...';
    }
    if (btn) btn.textContent = 'Record';
}

function formatKeyEvent(e) {
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    
    const key = getKeyName(e.code);
    if (key && !['Control', 'Alt', 'Shift', 'Meta'].includes(key)) {
        parts.push(key);
    }
    
    return parts.join('+');
}

function getKeyName(code) {
    if (code.startsWith('Key')) return code.slice(3);
    if (code.startsWith('Digit')) return code.slice(5);
    if (code.startsWith('Numpad')) return code;
    
    const map = {
        'Space': 'Space',
        'Enter': 'Enter',
        'Escape': 'Escape',
        'Tab': 'Tab',
        'Backspace': 'Backspace',
        'Delete': 'Delete',
        'Insert': 'Insert',
        'Home': 'Home',
        'End': 'End',
        'PageUp': 'PageUp',
        'PageDown': 'PageDown',
        'ArrowUp': 'Up',
        'ArrowDown': 'Down',
        'ArrowLeft': 'Left',
        'ArrowRight': 'Right',
        'F1': 'F1', 'F2': 'F2', 'F3': 'F3', 'F4': 'F4',
        'F5': 'F5', 'F6': 'F6', 'F7': 'F7', 'F8': 'F8',
        'F9': 'F9', 'F10': 'F10', 'F11': 'F11', 'F12': 'F12',
        'PrintScreen': 'PrintScreen',
        'Pause': 'Pause'
    };
    
    return map[code] || null;
}

async function saveHotkey() {
    const key = document.getElementById('hotkey-key')?.value;
    const pluginId = document.getElementById('hotkey-plugin')?.value;
    const action = document.getElementById('hotkey-action')?.value;
    const enabled = document.getElementById('hotkey-enabled')?.checked ?? true;
    
    if (!key || !pluginId) {
        return;
    }
    
    const hotkey = {
        id: state.editingHotkey?.id || `hk-${Date.now()}`,
        key,
        plugin_id: pluginId,
        action,
        enabled
    };
    
    if (state.editingHotkey) {
        const idx = state.hotkeys.findIndex(h => h.id === state.editingHotkey.id);
        if (idx !== -1) state.hotkeys[idx] = hotkey;
    } else {
        state.hotkeys.push(hotkey);
        state.selectedIndex = state.hotkeys.length - 1;
    }
    
    closeEditModal();
    renderList();
    updateSelection();
    
    await persistHotkeys();
}

async function deleteSelected() {
    if (state.selectedIndex < 0 || state.selectedIndex >= state.hotkeys.length) return;
    
    state.hotkeys.splice(state.selectedIndex, 1);
    state.selectedIndex = Math.min(state.selectedIndex, Math.max(0, state.hotkeys.length - 1));
    
    renderList();
    updateSelection();
    
    await persistHotkeys();
}

async function toggleSelected() {
    if (state.selectedIndex < 0 || state.selectedIndex >= state.hotkeys.length) return;
    
    state.hotkeys[state.selectedIndex].enabled = !state.hotkeys[state.selectedIndex].enabled;
    
    renderList();
    updateSelection();
    
    await persistHotkeys();
}

async function persistHotkeys() {
    try {
        await fetch('/api/hotkeys', {
            method: 'PUT',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ hotkeys: state.hotkeys })
        });
    } catch (error) {
        console.error('Failed to save hotkeys:', error);
    }
}

export function handleKey(e) {
    if (state.editModalOpen) {
        handleModalKey(e);
        return;
    }
    
    const handler = keyHandlers[e.key];
    if (handler) {
        e.preventDefault();
        handler();
    }
}

function handleModalKey(e) {
    if (state.recordingKey) {
        e.preventDefault();
        const key = formatKeyEvent(e);
        if (key && !['Ctrl', 'Alt', 'Shift', 'Super'].includes(key)) {
            stopKeyRecording(key);
        }
        return;
    }
    
    if (e.key === 'Escape') {
        e.preventDefault();
        closeEditModal();
    } else if (e.key === 'Enter') {
        e.preventDefault();
        saveHotkey();
    }
}

const keyHandlers = {
    ArrowUp: () => navigate(-1),
    ArrowDown: () => navigate(1),
    Enter: () => {
        if (state.hotkeys.length > 0 && state.selectedIndex >= 0) {
            openEditModal(state.hotkeys[state.selectedIndex]);
        }
    },
    a: () => openEditModal(),
    A: () => openEditModal(),
    d: deleteSelected,
    D: deleteSelected,
    ' ': toggleSelected
};

function navigate(delta) {
    const total = state.hotkeys.length;
    if (total === 0) return;
    
    const newIndex = Math.max(0, Math.min(total - 1, state.selectedIndex + delta));
    
    if (newIndex !== state.selectedIndex) {
        state.selectedIndex = newIndex;
        updateSelection();
    }
}

export function onFocus() {
    updateSelection();
}

export function onBlur() {
}

export function cleanup() {
    const list = document.getElementById('hotkeys-list');
    if (list) {
        list.removeEventListener('click', handleClick);
    }
}

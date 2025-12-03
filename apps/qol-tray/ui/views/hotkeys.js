export const id = 'hotkeys';

const state = {
    hotkeys: [],
    plugins: [],
    selectedIndex: -1,
    editModalOpen: false,
    recordingKey: false,
    editingHotkey: null,
    modalFieldIndex: 0
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
            const actionLabel = getActionLabel(plugin, hk.action);
            
            return `
                <div class="hotkey-row ${hk.enabled ? '' : 'disabled'}" data-index="${index}">
                    <span class="col-key"><kbd>${hk.key}</kbd></span>
                    <span class="col-plugin">${pluginName}</span>
                    <span class="col-action">${actionLabel}</span>
                    <span class="col-status">${hk.enabled ? '●' : '○'}</span>
                </div>
            `;
        }).join('')}
    `;
}

function getActionLabel(plugin, actionId) {
    if (!plugin) return actionId;
    const action = plugin.actions?.find(a => a.id === actionId);
    return action ? action.label : actionId;
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
    
    if (e.target.closest('#hotkey-key')) {
        startKeyRecording();
        return;
    }
}

function openEditModal(hotkey = null, keepPlugin = null) {
    state.editModalOpen = true;
    state.editingHotkey = hotkey;
    state.recordingKey = false;
    
    const isNew = !hotkey;
    const title = isNew ? 'Add Hotkey' : 'Edit Hotkey';
    const selectedPluginId = keepPlugin || hotkey?.plugin_id || '';
    
    const pluginOptions = state.plugins.map(p => 
        `<option value="${p.id}" ${selectedPluginId === p.id ? 'selected' : ''}>${p.name}</option>`
    ).join('');
    
    const modal = document.createElement('div');
    modal.className = 'edit-modal';
    modal.innerHTML = `
        <div class="edit-modal-content">
            <h3>${title}</h3>
            
            <div class="form-group">
                <label>Plugin</label>
                <select id="hotkey-plugin" tabindex="1">
                    <option value="">Select plugin...</option>
                    ${pluginOptions}
                </select>
            </div>
            
            <div class="form-group">
                <label>Action</label>
                <select id="hotkey-action" tabindex="2">
                    <option value="">Select plugin first...</option>
                </select>
            </div>
            
            <div class="form-group">
                <label>Shortcut <span class="hint">(Enter to record)</span></label>
                <div class="key-input-row">
                    <input type="text" id="hotkey-key" tabindex="3" value="${hotkey?.key || ''}" readonly placeholder="Press Enter to record">
                </div>
            </div>
            
            <div class="form-group">
                <label class="checkbox-label">
                    <input type="checkbox" id="hotkey-enabled" tabindex="4" ${hotkey?.enabled !== false ? 'checked' : ''}>
                    Enabled
                </label>
            </div>
            
            <div class="modal-buttons">
                <button class="modal-cancel" tabindex="5">Cancel <kbd>Esc</kbd></button>
                <button class="modal-save" tabindex="6">Save <kbd>Ctrl+S</kbd></button>
            </div>
        </div>
    `;
    
    container.appendChild(modal);
    
    modal.addEventListener('click', handleModalClick);
    
    const pluginSelect = document.getElementById('hotkey-plugin');
    pluginSelect.addEventListener('change', () => updateActionOptions(pluginSelect.value));
    
    if (selectedPluginId) {
        updateActionOptions(selectedPluginId, hotkey?.action);
    }
    
    setTimeout(() => {
        const pluginEl = document.getElementById('hotkey-plugin');
        if (pluginEl) {
            pluginEl.focus();
            state.modalFieldIndex = 0;
        }
    }, 0);
}

function getModalFields() {
    if (!state.editModalOpen) return [];
    const modal = container.querySelector('.edit-modal');
    if (!modal) return [];
    
    return [
        document.getElementById('hotkey-plugin'),
        document.getElementById('hotkey-action'),
        document.getElementById('hotkey-key'),
        document.getElementById('hotkey-enabled'),
        container.querySelector('.modal-cancel'),
        container.querySelector('.modal-save')
    ].filter(Boolean);
}

function getAssignedActions(pluginId) {
    return state.hotkeys
        .filter(h => h.plugin_id === pluginId && h.id !== state.editingHotkey?.id)
        .map(h => h.action);
}

function updateActionOptions(pluginId, selectedAction = null) {
    const actionSelect = document.getElementById('hotkey-action');
    if (!actionSelect) return;
    
    const plugin = state.plugins.find(p => p.id === pluginId);
    
    if (!plugin || !plugin.actions || plugin.actions.length === 0) {
        actionSelect.innerHTML = '<option value="run">Run</option>';
        return;
    }
    
    const assignedActions = getAssignedActions(pluginId);
    const availableActions = plugin.actions.filter(a => 
        !assignedActions.includes(a.id) || a.id === selectedAction
    );
    
    if (availableActions.length === 0) {
        actionSelect.innerHTML = '<option value="">All actions assigned</option>';
        return;
    }
    
    actionSelect.innerHTML = availableActions.map(a => 
        `<option value="${a.id}" ${selectedAction === a.id ? 'selected' : ''}>${a.label}</option>`
    ).join('');
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
    if (input) {
        input.placeholder = 'Press keys... (Esc to cancel)';
        input.value = '';
        input.classList.add('recording');
    }
}

function stopKeyRecording(key) {
    state.recordingKey = false;
    const input = document.getElementById('hotkey-key');
    if (input) {
        if (key) input.value = key;
        input.placeholder = 'Press Enter to record';
        input.classList.remove('recording');
    }
}

function formatKeyEvent(e) {
    const parts = [];
    if (e.ctrlKey) parts.push('Ctrl');
    if (e.altKey) parts.push('Alt');
    if (e.shiftKey) parts.push('Shift');
    if (e.metaKey) parts.push('Super');
    
    // Don't format if only modifiers are pressed
    if (['Control', 'Alt', 'Shift', 'Meta'].includes(e.key)) {
        if (parts.length > 0) return parts.join('+');
        return '';
    }
    
    const key = getKeyName(e.code);
    if (key) {
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
    
    if (!key || !pluginId || !action) {
        return;
    }
    
    const hotkey = {
        id: state.editingHotkey?.id || `hk-${Date.now()}`,
        key,
        plugin_id: pluginId,
        action,
        enabled
    };
    
    const isEditing = !!state.editingHotkey;
    
    if (isEditing) {
        const idx = state.hotkeys.findIndex(h => h.id === state.editingHotkey.id);
        if (idx !== -1) state.hotkeys[idx] = hotkey;
        closeEditModal();
    } else {
        state.hotkeys.push(hotkey);
        state.selectedIndex = state.hotkeys.length - 1;
        resetModalForNextHotkey(pluginId);
    }
    
    renderList();
    updateSelection();
    
    await persistHotkeys();
}

function resetModalForNextHotkey(pluginId) {
    const keyInput = document.getElementById('hotkey-key');
    if (keyInput) keyInput.value = '';
    
    updateActionOptions(pluginId);
    
    const actionSelect = document.getElementById('hotkey-action');
    if (actionSelect && actionSelect.value) {
        actionSelect.focus();
        state.modalFieldIndex = 1;
    } else {
        closeEditModal();
    }
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

function getModalContext() {
    const activeEl = document.activeElement;
    const fields = getModalFields();
    const currentIndex = fields.indexOf(activeEl);
    return { activeEl, fields, currentIndex };
}

function syncFieldIndex(ctx) {
    if (ctx.currentIndex !== -1) {
        state.modalFieldIndex = ctx.currentIndex;
    }
}

function handleRecordingKey(e) {
    e.preventDefault();
    e.stopPropagation();

    if (e.key === 'Escape') {
        stopKeyRecording(document.getElementById('hotkey-key')?.value || '');
        return true;
    }

    const MODIFIERS = ['Control', 'Alt', 'Shift', 'Meta'];
    if (MODIFIERS.includes(e.key)) {
        const current = formatKeyEvent(e);
        const input = document.getElementById('hotkey-key');
        if (input && current) {
            input.value = current;
        }
        return true;
    }

    const key = formatKeyEvent(e);
    const MODIFIER_NAMES = ['Ctrl', 'Alt', 'Shift', 'Super'];
    if (key && !MODIFIER_NAMES.includes(key)) {
        stopKeyRecording(key);
        focusNextField();
    }
    return true;
}

function handleModalNavigation(e, ctx) {
    if (e.key !== 'Tab') return false;

    e.preventDefault();
    e.stopPropagation();

    if (ctx.fields.length === 0) return true;

    const direction = e.shiftKey ? -1 : 1;
    state.modalFieldIndex = (state.modalFieldIndex + direction + ctx.fields.length) % ctx.fields.length;
    ctx.fields[state.modalFieldIndex]?.focus();
    return true;
}

const enterHandlers = [
    { match: el => el.id === 'hotkey-key', action: () => startKeyRecording() },
    { match: el => el.classList.contains('modal-save'), action: () => saveHotkey() },
    { match: el => el.classList.contains('modal-cancel'), action: () => closeEditModal() },
    { match: el => el.type === 'checkbox', action: el => { el.checked = !el.checked; } },
    { match: () => true, action: () => focusNextField() }
];

function handleModalAction(e, ctx) {
    if (e.key === 'Escape') {
        e.preventDefault();
        closeEditModal();
        return true;
    }

    if (e.key === 's' && e.ctrlKey && !e.altKey && !e.metaKey) {
        e.preventDefault();
        saveHotkey();
        return true;
    }

    if (e.key === 'Enter') {
        e.preventDefault();
        e.stopPropagation();
        const handler = enterHandlers.find(h => h.match(ctx.activeEl));
        handler.action(ctx.activeEl);
        return true;
    }

    if (e.key === ' ' && ctx.activeEl.type === 'checkbox') {
        return true;
    }

    return false;
}

function handleModalKey(e) {
    if (state.recordingKey) {
        handleRecordingKey(e);
        return;
    }

    const ctx = getModalContext();
    syncFieldIndex(ctx);

    if (handleModalAction(e, ctx)) return;
    if (handleModalNavigation(e, ctx)) return;
}

function focusNextField() {
    const fields = getModalFields();
    if (fields.length === 0) return;
    
    const nextIndex = (state.modalFieldIndex + 1) % fields.length;
    state.modalFieldIndex = nextIndex;
    fields[nextIndex]?.focus();
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

export function isBlocking() {
    return state.editModalOpen;
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

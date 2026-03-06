export function resolveModalKeyAction(e, activeEl) {
    if (e.key === 'Escape') { e.preventDefault(); return 'close'; }
    if (e.key === 'Enter' && e.ctrlKey) { e.preventDefault(); return 'save'; }
    if (e.key === 'Enter') { e.preventDefault(); return resolveEnterAction(activeEl); }
    if (e.key === 'Tab') { e.preventDefault(); return 'tab'; }
    return null;
}

function resolveEnterAction(activeEl) {
    if (activeEl?.id === 'hotkey-key') return 'startRecording';
    if (activeEl?.classList.contains('modal-cancel')) return 'close';
    if (activeEl?.classList.contains('modal-save')) return 'save';
    return 'advanceFocus';
}

export function applyModalAction(action, e, setEditModal, saveHotkey, modalFieldIndexRef, setModalFieldIndex) {
    if (action === 'close') { setEditModal(null); return; }
    if (action === 'save') { saveHotkey(); return; }
    if (action === 'startRecording') {
        setEditModal(prev => prev ? { ...prev, recording: true, key: '' } : prev);
        return;
    }
    if (action === 'advanceFocus') { advanceToNextField(); return; }
    if (action === 'tab') tabCycleFields(e.shiftKey ? -1 : 1, modalFieldIndexRef, setModalFieldIndex);
}

function advanceToNextField() {
    const fields = Array.from(document.querySelectorAll('.edit-modal [tabindex]'));
    const idx = fields.indexOf(document.activeElement);
    if (idx >= 0 && idx + 1 < fields.length) fields[idx + 1].focus();
}

function tabCycleFields(direction, modalFieldIndexRef, setModalFieldIndex) {
    const fields = Array.from(document.querySelectorAll('.edit-modal [tabindex]'));
    if (fields.length === 0) return;
    const next = (modalFieldIndexRef.current + direction + fields.length) % fields.length;
    setModalFieldIndex(next);
    fields[next]?.focus();
}

import { html } from '../../lib/html.js';
import { ModalActions } from '../../lib/components/ModalPreact.js';

export function ActionEditForm({ modal, fieldProps, handlers }) {
    const { updateField, onClose, onSave } = handlers;
    let fi = 0;
    return html`
        <div class="edit-modal-content">
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>ID <span class="hint">(used in API calls)</span></label>
                <input type="text" value=${modal.actionId} placeholder="e.g., open-vscode"
                       disabled=${!modal.isNew} onInput=${(e) => updateField('actionId', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Name</label>
                <input type="text" value=${modal.name} placeholder="e.g., Open in VS Code"
                       onInput=${(e) => updateField('name', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Description <span class="hint">(optional)</span></label>
                <input type="text" value=${modal.description} placeholder="e.g., Opens a path in Visual Studio Code"
                       onInput=${(e) => updateField('description', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Command <span class="hint">(use ${'{{'}param${'}}'}  for parameters)</span></label>
                <input type="text" value=${modal.command} placeholder="e.g., code {{path}}"
                       onInput=${(e) => updateField('command', e.target.value)} />
            </div>
            <div class="form-group" ...${fieldProps(fi++)}>
                <label>Timeout <span class="hint">(seconds)</span></label>
                <input type="number" value=${modal.timeout} min="1" max="3600"
                       onInput=${(e) => updateField('timeout', parseInt(e.target.value, 10) || 60)} />
            </div>
            <${ModalActions} onClose=${onClose} onSave=${onSave} />
        </div>
    `;
}


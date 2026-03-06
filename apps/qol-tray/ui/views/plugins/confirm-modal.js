import { html } from '../../lib/html.js';
import { Modal } from '../../components/ModalPreact.js';

export function UninstallConfirmModal({ plugin, pluginId, onClose, onConfirm }) {
    return html`
        <${Modal} open=${pluginId !== null} onClose=${onClose} className="confirm-modal">
            <div class="confirm-modal-content">
                <h3>Delete "${plugin?.name || pluginId}"?</h3>
                <p>This will uninstall the plugin and remove all its data.</p>
                <div class="confirm-modal-buttons">
                    <button class="btn btn-ghost confirm-cancel" onClick=${onClose}>Cancel (Esc)</button>
                    <button class="btn btn-danger confirm-delete" onClick=${onConfirm}>Delete (Enter)</button>
                </div>
            </div>
        <//>
    `;
}

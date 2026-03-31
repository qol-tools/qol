import { html } from '../../lib/html.js';
import { Modal, ModalFooter } from '../../components/ModalPreact.js';

export function UninstallConfirmModal({ plugin, pluginId, onClose, onConfirm }) {
    return html`
        <${Modal} open=${pluginId !== null} onClose=${onClose} dismissOnBackdrop=${true} className="confirm-modal">
            <div class="confirm-modal-content">
                <h3>Delete "${plugin?.name || pluginId}"?</h3>
                <p>This will uninstall the plugin and remove all its data.</p>
                <${ModalFooter} actions=${[
                    { label: 'Cancel', kbd: 'Esc', onClick: onClose },
                    { label: 'Delete', kbd: 'Enter', variant: 'btn-danger', onClick: onConfirm },
                ]} />
            </div>
        <//>
    `;
}

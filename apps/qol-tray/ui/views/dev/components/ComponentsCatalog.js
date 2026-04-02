import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { SurfaceContainer } from '../../../components/SurfaceContainer.js';
import { directSurfaces } from '../../../lib/surface-traits.js';
import { Modal, ModalFooter } from '../../../components/ModalPreact.js';
import { ToggleSwitch } from '../../../components/ToggleSwitch.js';
import { CustomSelect } from '../../../components/CustomSelect.js';
import { Expander, ExpanderTrigger, ExpanderBody } from '../../../components/Expander.js';
import { Badge, HealthDot, Alert } from '../../../components/StatusIndicators.js';

export function ComponentsCatalog() {
    return html`
        <div class="catalog">
            <${CatalogGroup} title="Display">
                <${ButtonShowcase} />
                <${StatusShowcase} />
            <//>
            <${CatalogGroup} title="Interactive">
                <${DropdownShowcase} />
                <${ExpanderShowcase} />
                <${ToggleShowcase} />
                <${ModalShowcase} />
                <${DepthDiver} />
            <//>
        </div>
    `;
}

function CatalogGroup({ title, children }) {
    return html`
        <div class="catalog-group">
            <div class="catalog-group-label">${title}</div>
            <div class="catalog-group-body">${children}</div>
        </div>
    `;
}

function CatalogSection({ title, children }) {
    return html`
        <div class="catalog-section">
            <div class="catalog-section-label">${title}</div>
            ${children}
        </div>
    `;
}

function CatalogRow({ label, children }) {
    return html`
        <div class="catalog-row">
            ${label && html`<div class="catalog-row-label">${label}</div>`}
            <div class="catalog-row-content">${children}</div>
        </div>
    `;
}

function ButtonShowcase() {
    return html`
        <${CatalogSection} title="Buttons">
            <${CatalogRow} label="Variants">
                <button class="btn" data-selected-surface="">Secondary</button>
                <button class="btn btn-primary" data-selected-surface="">Primary</button>
                <button class="btn btn-ghost" data-selected-surface="">Ghost</button>
                <button class="btn btn-danger" data-selected-surface="">Danger</button>
                <button class="btn" data-selected-surface="" disabled>Disabled</button>
            <//>
            <${CatalogRow} label="Small">
                <button class="btn btn-sm" data-selected-surface="">Secondary</button>
                <button class="btn btn-sm btn-primary" data-selected-surface="">Primary</button>
                <button class="btn btn-sm btn-ghost" data-selected-surface="">Ghost</button>
            <//>
            <${CatalogRow} label="With icons">
                <button class="btn" data-selected-surface=""><span class="btn-icon">${'\u2193'}</span> Pull</button>
                <button class="btn" data-selected-surface=""><span class="btn-icon">${'\u2191'}</span> Push</button>
                <button class="btn btn-primary" data-selected-surface=""><span class="btn-icon">${'\u26a1'}</span> Connect</button>
                <button class="btn btn-ghost" data-selected-surface="">Disconnect</button>
            <//>
        <//>
    `;
}

function DropdownShowcase() {
    const options = ['github', 'folder', 'local'];
    const labels = { github: 'GitHub', folder: 'Folder', local: 'Local' };
    const [value1, setValue1] = useState('github');
    const [value2, setValue2] = useState('folder');
    return html`
        <${CatalogSection} title="Dropdown">
            <${CatalogRow} label="Normal">
                <${CustomSelect} value=${value1} options=${options} labels=${labels} onChange=${setValue1} />
            <//>
            <${CatalogRow} label="Compact">
                <${CustomSelect} value=${value2} options=${options} labels=${labels} onChange=${setValue2} compact=${true} />
            <//>
        <//>
    `;
}

function ExpanderShowcase() {
    const [open1, setOpen1] = useState(false);
    const [open2, setOpen2] = useState(true);
    return html`
        <${CatalogSection} title="Expander">
            <${CatalogRow}>
                <${Expander} open=${open1} onToggle=${() => setOpen1(!open1)}>
                    <${ExpanderTrigger}>Collapsed<//>
                    <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span><//>
                <//>
                <${Expander} open=${open2} onToggle=${() => setOpen2(!open2)}>
                    <${ExpanderTrigger}>Expanded<//>
                    <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span><//>
                <//>
            <//>
        <//>
    `;
}

function ToggleShowcase() {
    const [toggle1, setToggle1] = useState(true);
    const [toggle2, setToggle2] = useState(false);
    return html`
        <${CatalogSection} title="Toggle">
            <${CatalogRow}>
                <span data-selected-surface="" onClick=${() => setToggle1(!toggle1)}>
                    <${ToggleSwitch} checked=${toggle1} onChange=${setToggle1} label="Enabled" />
                </span>
                <span data-selected-surface="" onClick=${() => setToggle2(!toggle2)}>
                    <${ToggleSwitch} checked=${toggle2} onChange=${setToggle2} label="Disabled" />
                </span>
            <//>
        <//>
    `;
}

function StatusShowcase() {
    return html`
        <${CatalogSection} title="Status">
            <${CatalogRow} label="Badge">
                <${Badge}>3<//>
                <${Badge} style=${{ background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }}>OK<//>
                <${Badge} style=${{ background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' }}>Review<//>
            <//>
            <${CatalogRow} label="Health dots">
                <span class="catalog-status-row"><${HealthDot} /> Not configured</span>
                <span class="catalog-status-row"><${HealthDot} health="healthy" /> Healthy</span>
                <span class="catalog-status-row"><${HealthDot} health="attention" /> Attention</span>
                <span class="catalog-status-row"><${HealthDot} health="error" /> Error</span>
            <//>
            <${CatalogRow} label="Alerts">
                <${Alert} variant="warning">Warning alert<//>
                <${Alert} variant="error">Error alert<//>
            <//>
        <//>
    `;
}

function ModalShowcase() {
    const [open, setOpen] = useState(false);
    const close = useCallback(() => setOpen(false), []);
    return html`
        <${CatalogSection} title="Modal">
            <${CatalogRow} label="Open modal">
                <button class="btn" data-selected-surface="" onClick=${() => setOpen(true)}>Open test modal</button>
            <//>
        <//>
        ${open && html`
            <${Modal} open=${true} onClose=${close} dismissOnBackdrop=${true} className="edit-modal">
                <div class="edit-modal-content">
                    <h3>Test Modal</h3>
                    <p style="color:var(--text-secondary); margin:var(--space-3) 0;">
                        This modal is a surface layer. The wedge should appear here.
                        Arrow keys navigate between buttons. ESC returns to the previous layer.
                    </p>
                    <${ModalFooter} actions=${[
                        { label: 'Close', kbd: 'Esc', onClick: close },
                        { label: 'Action', variant: 'btn-primary', onClick: () => {} },
                    ]} />
                </div>
            <//>
        `}
    `;
}

function DepthDiver() {
    return html`
        <${CatalogSection} title="Depth diver">
            <${CatalogRow} label="Enter to descend, ESC to ascend">
                <div class="depth-level-entry" data-selected-surface="">
                    <${DepthLevel} level=${1} />
                </div>
            <//>
        <//>
    `;
}

function depthDive(e) {
    const btn = e.currentTarget;
    const container = btn.closest('[data-surface-container]');
    const child = container?.querySelector('[data-surface-container]');
    if (!child) return;
    for (const s of directSurfaces(container)) s.removeAttribute('data-dive-source');
    btn.setAttribute('data-dive-source', '');
    const surface = child.querySelector('[data-selected-surface]');
    if (surface) surface.focus({ preventScroll: true });
}

function DepthLevel({ level }) {
    const label = `Level ${level}`;
    return html`
        <${SurfaceContainer} className="depth-level">
            <button class="btn btn-sm" data-selected-surface="" onClick=${depthDive}>${label} - A</button>
            <button class="btn btn-sm" data-selected-surface="" onClick=${depthDive}>${label} - B</button>
            ${level < 6 && html`
                <div class="depth-level-child">
                    <${DepthLevel} level=${level + 1} />
                </div>
            `}
        </${SurfaceContainer}>
    `;
}

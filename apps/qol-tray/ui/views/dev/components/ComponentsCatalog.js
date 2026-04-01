import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { SurfaceContainer } from '../../../components/SurfaceContainer.js';
import { directSurfaces } from '../../../lib/surface-traits.js';
import { Modal, ModalFooter } from '../../../components/ModalPreact.js';
import { CodeBlock } from '../../../components/CodeBlock.js';
import { ToggleSwitch } from '../../../components/ToggleSwitch.js';

export function ComponentsCatalog() {
    return html`
        <div class="catalog">
            <${ButtonShowcase} />
            <${DropdownShowcase} />
            <${ExpanderShowcase} />
            <${FormShowcase} />
            <${StatusShowcase} />
            <${ModalShowcase} />
            <${CodeBlockShowcase} />
            <${DepthDiver} />
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
    const [open, setOpen] = useState(false);
    return html`
        <${CatalogSection} title="Dropdown">
            <${CatalogRow} label="Click to toggle">
                <div style="position:relative; display:inline-flex;">
                    <button class="btn btn-dropdown" data-selected-surface=""
                        aria-expanded=${open ? 'true' : 'false'}
                        onClick=${() => setOpen(!open)}>GitHub</button>
                    ${open && html`
                        <div class="catalog-dropdown-menu" onClick=${() => setOpen(false)}>
                            <div class="catalog-dropdown-item is-active">${'\u2713'} GitHub</div>
                            <div class="catalog-dropdown-item">${'\u00a0\u00a0'} Folder</div>
                        </div>
                    `}
                </div>
                <button class="btn btn-dropdown" data-selected-surface="">Folder</button>
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
                <div style="display:flex; gap:var(--space-3); align-items:flex-start; flex-wrap:wrap;">
                    <div class="btn btn-ghost btn-expander" data-selected-surface=""
                        aria-expanded=${open1 ? 'true' : 'false'}
                        onClick=${(e) => { if (e.target.closest('.btn-expander-body')) return; setOpen1(!open1); }}>
                        <div class="btn-expander-trigger"><span class="btn-icon btn-icon-chevron">${'\u25b6'}</span> Collapsed</div>
                        <div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>
                            <span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span>
                        </div>
                    </div>
                    <div class="btn btn-ghost btn-expander" data-selected-surface=""
                        aria-expanded=${open2 ? 'true' : 'false'}
                        onClick=${(e) => { if (e.target.closest('.btn-expander-body')) return; setOpen2(!open2); }}>
                        <div class="btn-expander-trigger"><span class="btn-icon btn-icon-chevron">${'\u25b6'}</span> Expanded</div>
                        <div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>
                            <span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span>
                        </div>
                    </div>
                </div>
            <//>
        <//>
    `;
}

function FormShowcase() {
    const [toggle1, setToggle1] = useState(true);
    const [toggle2, setToggle2] = useState(false);
    return html`
        <${CatalogSection} title="Form primitives">
            <${CatalogRow} label="ToggleSwitch">
                <span data-selected-surface="" onClick=${() => setToggle1(!toggle1)}>
                    <${ToggleSwitch} checked=${toggle1} onChange=${setToggle1} label="Enabled" />
                </span>
                <span data-selected-surface="" onClick=${() => setToggle2(!toggle2)}>
                    <${ToggleSwitch} checked=${toggle2} onChange=${setToggle2} label="Disabled" />
                </span>
            <//>
            <${CatalogRow} label="Badge">
                <span class="badge" data-selected-surface="">3</span>
                <span class="badge" data-selected-surface="" style="background:rgba(var(--success-rgb),0.14); border-color:rgba(var(--success-rgb),0.26);">OK</span>
                <span class="badge" data-selected-surface="" style="background:rgba(var(--warning-rgb),0.16); border-color:rgba(var(--warning-rgb),0.3);">Review</span>
            <//>
        <//>
    `;
}

function StatusShowcase() {
    return html`
        <${CatalogSection} title="Status indicators">
            <${CatalogRow} label="Health dots">
                <span class="catalog-status-row" data-selected-surface="">
                    <span class="profile-health-dot"></span> Not configured
                </span>
                <span class="catalog-status-row" data-selected-surface="">
                    <span class="profile-health-dot" data-health="healthy"></span> Healthy
                </span>
                <span class="catalog-status-row" data-selected-surface="">
                    <span class="profile-health-dot" data-health="attention"></span> Attention
                </span>
                <span class="catalog-status-row" data-selected-surface="">
                    <span class="profile-health-dot" data-health="error"></span> Error
                </span>
            <//>
            <${CatalogRow} label="Alerts">
                <div class="profile-sync-alert" data-selected-surface="" data-variant="warning" style="font-size:var(--fs-sm); padding:var(--space-2) var(--space-3);">Warning alert</div>
                <div class="profile-sync-alert" data-selected-surface="" data-variant="error" style="font-size:var(--fs-sm); padding:var(--space-2) var(--space-3);">Error alert</div>
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
                    <${CodeBlock} text=${"Layer depth test\nThis is inside a modal"} />
                    <${ModalFooter} actions=${[
                        { label: 'Close', kbd: 'Esc', onClick: close },
                        { label: 'Action', variant: 'btn-primary', onClick: () => {} },
                    ]} />
                </div>
            <//>
        `}
    `;
}

function CodeBlockShowcase() {
    return html`
        <${CatalogSection} title="CodeBlock">
            <${CatalogRow} label="Click to copy">
                <div data-selected-surface="" style="flex:1;">
                    <${CodeBlock} text=${"const x = 42;\nconsole.log(x);"} />
                </div>
            <//>
        <//>
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
    for (const s of directSurfaces(container)) s.setAttribute('data-selected', 'false');
    btn.setAttribute('data-selected', 'true');
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
                <div class="depth-level-child" data-selected-surface="">
                    <span class="depth-level-label">${'\u25b6'} ${label} - Dive deeper</span>
                    <${DepthLevel} level=${level + 1} />
                </div>
            `}
        </${SurfaceContainer}>
    `;
}

import { html } from '../../../lib/html.js';
import { useState } from 'preact/hooks';

export function ComponentsCatalog() {
    return html`
        <div class="catalog">
            <${ButtonShowcase} />
            <${DropdownShowcase} />
            <${ExpanderShowcase} />
            <${FormShowcase} />
            <${StatusShowcase} />
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

function S(props) {
    return html`<span data-selected-surface="" data-selected="false" ...${props} />`;
}

function ButtonShowcase() {
    return html`
        <${CatalogSection} title="Buttons">
            <${CatalogRow} label="Variants">
                <button class="btn" data-selected-surface="" data-selected="false">Secondary</button>
                <button class="btn btn-primary" data-selected-surface="" data-selected="false">Primary</button>
                <button class="btn btn-ghost" data-selected-surface="" data-selected="false">Ghost</button>
                <button class="btn btn-danger" data-selected-surface="" data-selected="false">Danger</button>
                <button class="btn" data-selected-surface="" data-selected="false" disabled>Disabled</button>
            <//>
            <${CatalogRow} label="Small">
                <button class="btn btn-sm" data-selected-surface="" data-selected="false">Secondary</button>
                <button class="btn btn-sm btn-primary" data-selected-surface="" data-selected="false">Primary</button>
                <button class="btn btn-sm btn-ghost" data-selected-surface="" data-selected="false">Ghost</button>
            <//>
            <${CatalogRow} label="With icons">
                <button class="btn" data-selected-surface="" data-selected="false"><span class="btn-icon">${'\u2193'}</span> Pull</button>
                <button class="btn" data-selected-surface="" data-selected="false"><span class="btn-icon">${'\u2191'}</span> Push</button>
                <button class="btn btn-primary" data-selected-surface="" data-selected="false"><span class="btn-icon">${'\u26a1'}</span> Connect</button>
                <button class="btn btn-ghost" data-selected-surface="" data-selected="false">Disconnect</button>
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
                    <button class="btn btn-dropdown" data-selected-surface="" data-selected="false"
                        aria-expanded=${open ? 'true' : 'false'}
                        onClick=${() => setOpen(!open)}>GitHub</button>
                    ${open && html`
                        <div class="catalog-dropdown-menu" onClick=${() => setOpen(false)}>
                            <div class="catalog-dropdown-item is-active">${'\u2713'} GitHub</div>
                            <div class="catalog-dropdown-item">${'\u00a0\u00a0'} Folder</div>
                        </div>
                    `}
                </div>
                <button class="btn btn-dropdown" data-selected-surface="" data-selected="false">Folder</button>
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
                    <div class="btn btn-ghost btn-expander" data-selected-surface="" data-selected="false"
                        aria-expanded=${open1 ? 'true' : 'false'}
                        onClick=${(e) => { if (e.target.closest('.btn-expander-body')) return; setOpen1(!open1); }}>
                        <div class="btn-expander-trigger"><span class="btn-icon btn-icon-chevron">${'\u25b6'}</span> Collapsed</div>
                        <div class="btn-expander-body" onClick=${(e) => e.stopPropagation()}>
                            <span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span>
                        </div>
                    </div>
                    <div class="btn btn-ghost btn-expander" data-selected-surface="" data-selected="false"
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
            <${CatalogRow} label="Toggle">
                <div class="toggle-inline" data-selected-surface="" data-selected="false" onClick=${() => setToggle1(!toggle1)}>
                    <div class=${`toggle-track ${toggle1 ? 'on' : ''}`} role="switch" aria-checked=${toggle1}><div class="toggle-thumb"></div></div>
                    <span class="toggle-inline-label">Enabled</span>
                </div>
                <div class="toggle-inline" data-selected-surface="" data-selected="false" onClick=${() => setToggle2(!toggle2)}>
                    <div class=${`toggle-track ${toggle2 ? 'on' : ''}`} role="switch" aria-checked=${toggle2}><div class="toggle-thumb"></div></div>
                    <span class="toggle-inline-label">Disabled</span>
                </div>
            <//>
            <${CatalogRow} label="Badge">
                <span class="badge" data-selected-surface="" data-selected="false">3</span>
                <span class="badge" data-selected-surface="" data-selected="false" style="background:rgba(var(--success-rgb),0.14); border-color:rgba(var(--success-rgb),0.26);">OK</span>
                <span class="badge" data-selected-surface="" data-selected="false" style="background:rgba(var(--warning-rgb),0.16); border-color:rgba(var(--warning-rgb),0.3);">Review</span>
            <//>
        <//>
    `;
}

function StatusShowcase() {
    return html`
        <${CatalogSection} title="Status indicators">
            <${CatalogRow} label="Health dots">
                <span class="catalog-status-row" data-selected-surface="" data-selected="false">
                    <span class="profile-health-dot"></span> Not configured
                </span>
                <span class="catalog-status-row" data-selected-surface="" data-selected="false">
                    <span class="profile-health-dot" data-health="healthy"></span> Healthy
                </span>
                <span class="catalog-status-row" data-selected-surface="" data-selected="false">
                    <span class="profile-health-dot" data-health="attention"></span> Attention
                </span>
                <span class="catalog-status-row" data-selected-surface="" data-selected="false">
                    <span class="profile-health-dot" data-health="error"></span> Error
                </span>
            <//>
            <${CatalogRow} label="Alerts">
                <div class="profile-sync-alert" data-selected-surface="" data-selected="false" data-variant="warning" style="font-size:var(--fs-sm); padding:var(--space-2) var(--space-3);">Warning alert</div>
                <div class="profile-sync-alert" data-selected-surface="" data-selected="false" data-variant="error" style="font-size:var(--fs-sm); padding:var(--space-2) var(--space-3);">Error alert</div>
            <//>
        <//>
    `;
}

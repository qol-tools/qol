import { html } from '../../../lib/html.js';
import { useCallback, useState } from 'preact/hooks';
import { SurfaceContainer } from '../../../components/SurfaceContainer.js';
import { directSurfaces } from '../../../lib/surface-traits.js';
import { Modal, ModalFooter } from '../../../components/ModalPreact.js';
import { ToggleSwitch } from '../../../components/ToggleSwitch.js';
import { CustomSelect } from '../../../components/CustomSelect.js';
import { Expander, ExpanderTrigger, ExpanderBody } from '../../../components/Expander.js';
import { Badge, HealthDot, Alert } from '../../../components/StatusIndicators.js';
import { ListGroup } from '../../../components/ListRow.js';
import { Surface } from '../../../components/Surface.js';
import { useListSelection } from '../../../hooks/useListSelection.js';
import { PluginRow } from '../../../components/rows/PluginRow.js';
import { LogRow, LogDetailModal } from '../../../components/rows/LogRow.js';
import { SuppressedRow } from '../../../components/rows/SuppressedRow.js';
import { BackupRow } from '../../../components/rows/BackupRow.js';

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
            <${CatalogGroup} title="Rows" inline=${false}>
                <${PluginRowShowcase} />
                <${LogRowShowcase} />
                <${SuppressedRowShowcase} />
                <${BackupRowShowcase} />
            <//>
        </div>
    `;
}

function CatalogGroup({ title, inline = true, children }) {
    return html`
        <div class="catalog-group">
            <div class="catalog-group-label">${title}</div>
            <div class=${inline ? 'catalog-group-body' : 'catalog-group-stack'}>${children}</div>
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
                <${Surface} as="button" className="btn">Secondary<//>
                <${Surface} as="button" className="btn btn-primary">Primary<//>
                <${Surface} as="button" className="btn btn-ghost">Ghost<//>
                <${Surface} as="button" className="btn btn-danger">Danger<//>
                <${Surface} as="button" className="btn" disabled>Disabled<//>
            <//>
            <${CatalogRow} label="Small">
                <${Surface} as="button" className="btn btn-sm">Secondary<//>
                <${Surface} as="button" className="btn btn-sm btn-primary">Primary<//>
                <${Surface} as="button" className="btn btn-sm btn-ghost">Ghost<//>
            <//>
            <${CatalogRow} label="With icons">
                <${Surface} as="button" className="btn"><span class="btn-icon">${'\u2193'}</span> Pull<//>
                <${Surface} as="button" className="btn"><span class="btn-icon">${'\u2191'}</span> Push<//>
                <${Surface} as="button" className="btn btn-primary"><span class="btn-icon">${'\u26a1'}</span> Connect<//>
                <${Surface} as="button" className="btn btn-ghost">Disconnect<//>
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
                <${Surface} as="span" onActivate=${() => setToggle1(!toggle1)}>
                    <${ToggleSwitch} checked=${toggle1} onChange=${setToggle1} label="Enabled" />
                <//>
                <${Surface} as="span" onActivate=${() => setToggle2(!toggle2)}>
                    <${ToggleSwitch} checked=${toggle2} onChange=${setToggle2} label="Disabled" />
                <//>
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
                <${Surface} as="button" className="btn" onActivate=${() => setOpen(true)}>Open test modal<//>
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
                <${Surface} className="depth-level-entry">
                    <${DepthLevel} level=${1} />
                <//>
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
            <${Surface} as="button" className="btn btn-sm" onActivate=${depthDive}>${label} - A<//>
            <${Surface} as="button" className="btn btn-sm" onActivate=${depthDive}>${label} - B<//>
            ${level < 6 && html`
                <div class="depth-level-child">
                    <${DepthLevel} level=${level + 1} />
                </div>
            `}
        </${SurfaceContainer}>
    `;
}

function PluginRowShowcase() {
    const sel = useListSelection();
    const [linked, setLinked] = useState({ 0: true, 1: false, 2: false });
    const toggleLink = (i) => () => setLinked(prev => ({ ...prev, [i]: !prev[i] }));
    const status = (i) => linked[i] ? 'linked' : (i === 1 ? 'local' : undefined);
    return html`
        <${CatalogSection} title="Plugin-style">
            <${ListGroup} onDeselect=${sel.deselect}>
                <${PluginRow} name="qol-window-actions" path="~/repos/qol-tools/qol-window-actions"
                    status=${status(0)} index=${0} selected=${sel.selected(0)} onSelect=${sel.select} onActivate=${toggleLink(0)} />
                <${PluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                    status=${status(1)} index=${1} selected=${sel.selected(1)} onSelect=${sel.select} onActivate=${toggleLink(1)} />
                <${PluginRow} name="qol-fx" path="~/repos/qol-tools/qol-fx"
                    status=${status(2)} index=${2} selected=${sel.selected(2)} onSelect=${sel.select} onActivate=${toggleLink(2)} />
            <//>
        <//>
    `;
}

function LogRowShowcase() {
    const sel = useListSelection();
    const [modalEntry, setModalEntry] = useState(null);
    const close = useCallback(() => setModalEntry(null), []);
    const openDetail = (src, msg, loc) => () => setModalEntry({ src, msg, loc });
    return html`
        <${CatalogSection} title="Log-style">
            <${ListGroup} onDeselect=${sel.deselect}>
                <${LogRow} time="14:32:01" level="startup" src="qol-window-actions" msg="Plugin initialized successfully"
                    index=${0} selected=${sel.selected(0)} onSelect=${sel.select}
                    onActivate=${openDetail('qol-window-actions', 'Plugin initialized successfully')} />
                <${LogRow} time="14:32:05" level="error" src="qol-alt-tab" loc="src/main.rs:42" count=${3}
                    msg="Failed to register hotkey: already registered by another process"
                    index=${1} selected=${sel.selected(1)} onSelect=${sel.select}
                    onActivate=${openDetail('qol-alt-tab', 'Failed to register hotkey: already registered by another process', 'src/main.rs:42')} />
                <${LogRow} time="14:32:08" level="suppressed" src="qol-fx" msg="Animation frame dropped (vsync miss)"
                    index=${2} selected=${sel.selected(2)} onSelect=${sel.select}
                    onActivate=${openDetail('qol-fx', 'Animation frame dropped (vsync miss)')} />
            <//>
        <//>
        ${modalEntry && html`<${LogDetailModal} entry=${modalEntry} onClose=${close} />`}
    `;
}

function SuppressedRowShowcase() {
    const sel = useListSelection();
    const [expanded, setExpanded] = useState(false);
    return html`
        <${CatalogSection} title="Card-style">
            <${ListGroup} className="list-group-cards" onDeselect=${sel.deselect}>
                <${SuppressedRow} sigKey="qol-alt-tab::hotkey_register_failed" count=${12}
                    msg="Failed to register hotkey: already registered by another process"
                    expanded=${expanded} onToggle=${() => setExpanded(!expanded)} onUnsuppress=${() => {}}
                    index=${0} selected=${sel.selected(0)} onSelect=${sel.select} />
                <${SuppressedRow} sigKey="qol-fx::vsync_miss" count=${47}
                    index=${1} selected=${sel.selected(1)} onSelect=${sel.select} onUnsuppress=${() => {}} />
                <${SuppressedRow} sigKey="qol-alt-tab::event_loop_stall" count=${312}
                    index=${2} selected=${sel.selected(2)} onSelect=${sel.select} onUnsuppress=${() => {}} />
            <//>
        <//>
    `;
}

function BackupRowShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Backup-style">
            <${ListGroup} onDeselect=${sel.deselect}>
                <${BackupRow} time="2026-04-02 14:30" fileName="profile-backup-2026-04-02T143001.toml" size="2.4 KB" review=${true}
                    index=${0} selected=${sel.selected(0)} onSelect=${sel.select} />
                <${BackupRow} time="2026-04-01 09:15" fileName="profile-backup-2026-04-01T091500.toml" size="1.8 KB"
                    index=${1} selected=${sel.selected(1)} onSelect=${sel.select} />
            <//>
        <//>
    `;
}

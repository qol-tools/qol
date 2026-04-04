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
import { Table, TableHeader, TableCell } from '../../../components/TableRow.js';
import { Surface } from '../../../components/Surface.js';
import { Button, RefreshButton } from '../../../components/Button.js';
import { EmptyState } from '../../../components/EmptyState.js';
import { useListSelection } from '../../../hooks/useListSelection.js';
import { LogRow, LogDetailModal } from '../../../components/rows/LogRow.js';
import { SuppressedRow } from '../../../components/rows/SuppressedRow.js';
import { BackupRow } from '../../../components/rows/BackupRow.js';
import { HotkeyRow } from '../../../components/rows/HotkeyRow.js';
import { ShortcutRow } from '../../../components/rows/ShortcutRow.js';
import { DevPluginRow } from '../../../components/rows/DevPluginRow.js';
import { StoreCard, StoreCardGrid } from '../../../components/rows/StoreCard.js';

export function ComponentsCatalog({ activeId }) {
    const showcases = {
        'buttons': ButtonShowcase,
        'status': StatusShowcase,
        'spinner': SpinnerShowcase,
        'empty-state': EmptyStateShowcase,
        'dropdown': DropdownShowcase,
        'expander': ExpanderShowcase,
        'toggle': ToggleShowcase,
        'modal': ModalShowcase,
        'depth-diver': DepthDiver,
        'dev-plugin-row': DevPluginRowShowcase,
        'log-row': LogRowShowcase,
        'suppressed-row': SuppressedRowShowcase,
        'backup-row': BackupRowShowcase,
        'hotkey-row': HotkeyTableShowcase,
        'shortcut-row': ShortcutTableShowcase,
        'store-card': StoreCardShowcase,
    };
    const Showcase = showcases[activeId];
    if (!Showcase) return null;
    return html`<div class="catalog"><${Showcase} /></div>`;
}

function CatalogSection({ title, children }) {
    return html`
        <div class="catalog-section">
            <div class="catalog-section-label">${title}</div>
            ${children}
        </div>
    `;
}

function StateLabel({ children }) {
    return html`<div class="catalog-state-label">${children}</div>`;
}

// ---------------------------------------------------------------------------
// Display
// ---------------------------------------------------------------------------

function ButtonShowcase() {
    return html`
        <${CatalogSection} title="Buttons">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${Button} variant="btn-primary" onActivate=${() => {}}>Interactive<//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>variants<//>
                    <div class="catalog-state-inline">
                        <${Button}>Secondary<//> <${Button} variant="btn-primary">Primary<//> <${Button} variant="btn-ghost">Ghost<//> <${Button} variant="btn-danger">Danger<//> <${Button} disabled>Disabled<//>
                    </div>
                    <${StateLabel}>small<//>
                    <div class="catalog-state-inline">
                        <${Button} small>Secondary<//> <${Button} small variant="btn-primary">Primary<//> <${Button} small variant="btn-ghost">Ghost<//>
                    </div>
                </div>
            </div>
        <//>
    `;
}

function StatusShowcase() {
    return html`
        <${CatalogSection} title="Status indicators">
            <div class="catalog-showcase">
                <div class="catalog-try" inert>
                    <div class="catalog-state-inline">
                        <${Badge}>3<//>
                        <${Badge} style=${{ background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }}>OK<//>
                        <${Badge} style=${{ background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' }}>Review<//>
                    </div>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>health dots<//>
                    <div class="catalog-state-inline">
                        <span class="catalog-status-row"><${HealthDot} /> None</span>
                        <span class="catalog-status-row"><${HealthDot} health="healthy" /> Healthy</span>
                        <span class="catalog-status-row"><${HealthDot} health="attention" /> Attention</span>
                        <span class="catalog-status-row"><${HealthDot} health="error" /> Error</span>
                    </div>
                    <${StateLabel}>alerts<//>
                    <${Alert} variant="warning">Warning alert<//>
                    <${Alert} variant="error">Error alert<//>
                </div>
            </div>
        <//>
    `;
}

function SpinnerShowcase() {
    return html`
        <${CatalogSection} title="Spinner">
            <div class="catalog-showcase">
                <div class="catalog-try" inert><${RefreshButton} /></div>
                <div class="catalog-states" inert>
                    <${StateLabel}>idle<//>
                    <${RefreshButton} />
                    <${StateLabel}>spinning<//>
                    <${RefreshButton} spinning />
                </div>
            </div>
        <//>
    `;
}

function EmptyStateShowcase() {
    return html`
        <${CatalogSection} title="Empty state">
            <div class="catalog-showcase">
                <div class="catalog-try" inert>
                    <${EmptyState} message="No items found" hint="Try adjusting your filters" />
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>default<//>
                    <span style="color:var(--text-muted); font-size:var(--fs-sm)">Single variant — message + hint</span>
                </div>
            </div>
        <//>
    `;
}

// ---------------------------------------------------------------------------
// Interactive
// ---------------------------------------------------------------------------

function DropdownShowcase() {
    const options = ['github', 'folder', 'local'];
    const labels = { github: 'GitHub', folder: 'Folder', local: 'Local' };
    const [value, setValue] = useState('github');
    return html`
        <${CatalogSection} title="Dropdown">
            <div class="catalog-showcase">
                <div class="catalog-try"><${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${setValue} /></div>
                <div class="catalog-states" inert>
                    <${StateLabel}>compact<//>
                    <${CustomSelect} value="folder" options=${options} labels=${labels} onChange=${() => {}} compact=${true} />
                </div>
            </div>
        <//>
    `;
}

function ExpanderShowcase() {
    const [open, setOpen] = useState(false);
    return html`
        <${CatalogSection} title="Expander">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${Expander} open=${open} onToggle=${() => setOpen(!open)}>
                        <${ExpanderTrigger}>${open ? 'Expanded' : 'Collapsed'}<//>
                        <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span><//>
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>collapsed<//>
                    <${Expander} open=${false} onToggle=${() => {}}>
                        <${ExpanderTrigger}>Section A<//>
                    <//>
                    <${StateLabel}>expanded<//>
                    <${Expander} open=${true} onToggle=${() => {}}>
                        <${ExpanderTrigger}>Section B<//>
                        <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Visible content.</span><//>
                    <//>
                </div>
            </div>
        <//>
    `;
}

function ToggleShowcase() {
    const [on, setOn] = useState(true);
    return html`
        <${CatalogSection} title="Toggle">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${ToggleSwitch} checked=${on} onChange=${setOn} label="Interactive toggle" />
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>on<//>
                    <${ToggleSwitch} checked=${true} onChange=${() => {}} label="Enabled" />
                    <${StateLabel}>off<//>
                    <${ToggleSwitch} checked=${false} onChange=${() => {}} label="Disabled" />
                    <${StateLabel}>with description<//>
                    <${ToggleSwitch} checked=${true} onChange=${() => {}} label="Push on change" description="Automatically sync when profile changes" />
                </div>
            </div>
        <//>
    `;
}

function ModalShowcase() {
    const [open, setOpen] = useState(false);
    const close = useCallback(() => setOpen(false), []);
    return html`
        <${CatalogSection} title="Modal">
            <${Button} onActivate=${() => setOpen(true)}>Open test modal<//>
        <//>
        ${open && html`
            <${Modal} open=${true} onClose=${close} dismissOnBackdrop=${true} className="edit-modal">
                <div class="edit-modal-content">
                    <h3>Test Modal</h3>
                    <p style="color:var(--text-secondary); margin:var(--space-3) 0;">
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
            <${Surface} className="depth-level-entry">
                <${DepthLevel} level=${1} />
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
            <${Button} small onActivate=${depthDive}>${label} - A<//>
            <${Button} small onActivate=${depthDive}>${label} - B<//>
            ${level < 6 && html`
                <div class="depth-level-child">
                    <${DepthLevel} level=${level + 1} />
                </div>
            `}
        </${SurfaceContainer}>
    `;
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

function DevPluginRowShowcase() {
    const sel = useListSelection();
    const [state, setState] = useState('rebuild');
    const cycle = () => {
        const order = ['linked', 'rebuild', 'linking', 'local'];
        setState(prev => order[(order.indexOf(prev) + 1) % order.length]);
    };
    const rebuildIcon = (cls) => html`<img class="plugin-action-rebuild-icon ${cls}" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true" />`;
    const actions = state === 'rebuild'
        ? [{ label: 'Rebuild', run: cycle }, { label: 'Mute Logs', run: () => {} }, { label: 'Edit Filters', run: () => {} }]
        : [{ label: state === 'linked' ? 'Unlink' : 'Link', run: cycle }, { label: 'Mute Logs', run: () => {} }, { label: 'Edit Filters', run: () => {} }];
    const icon = state === 'linking'
        ? html`<span class="refresh-btn spinning" aria-hidden="true"></span>`
        : rebuildIcon(state === 'rebuild' ? 'has-rebuild' : 'rebuild-idle');

    return html`
        <${CatalogSection} title="Dev plugin row">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <div class="plugin-list">
                        <${DevPluginRow} name="qol-window-actions" path="~/repos/qol-tools/qol-window-actions"
                            status=${state === 'local' ? 'local' : 'linked'} pluginId="plugin-window-actions"
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select}
                            actions=${state === 'linking' ? [] : actions} actionIcon=${icon}
                            className=${state === 'linking' ? 'is-linking' : undefined}
                            badges=${html`<${Badge} style=${{ background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }}>${state}<//>`}
                            meta=${html`<span style="font-size:var(--fs-xs); color:var(--text-faint)">fp a3b2c1d0 • Built 2m ago</span>`} />
                    </div>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>linked (idle)<//>
                    <div class="plugin-list"><${DevPluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                        status="linked" actionIcon=${rebuildIcon('rebuild-idle')} /></div>
                    <${StateLabel}>rebuild needed (glow)<//>
                    <div class="plugin-list"><${DevPluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                        status="linked" actionIcon=${rebuildIcon('has-rebuild')}
                        meta=${html`<span style="font-size:var(--fs-xs); color:var(--text-faint)">Source changed • fp a3b2c1d0</span>`} /></div>
                    <${StateLabel}>linking (spin)<//>
                    <div class="plugin-list"><${DevPluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                        status="linked" className="is-linking"
                        actionIcon=${html`<span class="refresh-btn spinning" aria-hidden="true"></span>`} /></div>
                    <${StateLabel}>local<//>
                    <div class="plugin-list"><${DevPluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                        status="local" actionIcon=${rebuildIcon('rebuild-idle')}
                        badges=${html`<${Badge} style=${{ background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' }}>Local<//>`} /></div>
                </div>
            </div>
        <//>
    `;
}

function LogRowShowcase() {
    const sel = useListSelection();
    const [modalEntry, setModalEntry] = useState(null);
    const close = useCallback(() => setModalEntry(null), []);
    return html`
        <${CatalogSection} title="Log row">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${ListGroup} onDeselect=${sel.deselect}>
                        <${LogRow} time="14:32:05" level="error" src="qol-alt-tab" loc="src/main.rs:42" count=${3} severity="warning"
                            msg="Failed to register hotkey: already registered"
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select}
                            onActivate=${() => setModalEntry({ src: 'qol-alt-tab', msg: 'Failed to register hotkey', loc: 'src/main.rs:42' })} />
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>startup<//>
                    <${LogRow} time="14:32:01" level="startup" src="qol-window-actions" msg="Plugin initialized successfully" />
                    <${StateLabel}>error + count<//>
                    <${LogRow} time="14:32:05" level="error" src="qol-alt-tab" loc="src/main.rs:42" count=${3} severity="warning" msg="Failed to register hotkey" />
                    <${StateLabel}>suppressed<//>
                    <${LogRow} time="14:32:08" level="suppressed" src="qol-fx" msg="Animation frame dropped (vsync miss)" />
                </div>
            </div>
        <//>
        ${modalEntry && html`<${LogDetailModal} entry=${modalEntry} onClose=${close} />`}
    `;
}

function SuppressedRowShowcase() {
    const sel = useListSelection();
    const [expanded, setExpanded] = useState(false);
    const entry = {
        count: 12, last_message: 'Failed to register hotkey: already registered by another process',
        source: 'qol-alt-tab', location: 'src/main.rs:42',
        first_seen: '2026-04-02T14:30:01', last_seen: '2026-04-02T14:32:05',
    };
    return html`
        <${CatalogSection} title="Suppressed row">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${ListGroup} className="logs-suppressed-list" onDeselect=${sel.deselect}>
                        <${SuppressedRow} sigKey="qol-alt-tab::hotkey_register_failed" entry=${entry}
                            expanded=${expanded} index=${0} selected=${sel.selected(0)}
                            onSelect=${sel.select} onToggle=${() => setExpanded(e => !e)} onUnsuppress=${() => {}} />
                    <//>
                </div>
                <div class="catalog-states logs-suppressed-list" inert>
                    <${StateLabel}>collapsed<//>
                    <${SuppressedRow} sigKey="qol-fx::vsync_miss" entry=${{ count: 47, last_message: 'Animation frame dropped' }} expanded=${false} onToggle=${() => {}} />
                    <${StateLabel}>expanded<//>
                    <${SuppressedRow} sigKey="qol-alt-tab::event_loop_stall" entry=${{ count: 312, source: 'qol-alt-tab', first_seen: '2026-04-01T08:00:00', last_seen: '2026-04-02T14:32:00' }} expanded=${true} onToggle=${() => {}} />
                </div>
            </div>
        <//>
    `;
}

function BackupRowShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Backup row">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${ListGroup} onDeselect=${sel.deselect}>
                        <${BackupRow} time="2026-04-02 14:30" fileName="profile-backup-2026-04-02T143001.toml" size="2.4 KB" review=${true}
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select} />
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>with review<//>
                    <${BackupRow} time="2026-04-02 14:30" fileName="profile-backup.toml" size="2.4 KB" review=${true} />
                    <${StateLabel}>without review<//>
                    <${BackupRow} time="2026-04-01 09:15" fileName="profile-backup.toml" size="1.8 KB" />
                </div>
            </div>
        <//>
    `;
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

function HotkeyTableShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Hotkey table">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${Table} columns="8rem 1fr 1fr" onDeselect=${sel.deselect}>
                        <${TableHeader}><${TableCell}>Shortcut<//><${TableCell}>Plugin<//><${TableCell}>Action<//><//>
                        <${HotkeyRow} shortcut="Alt+Tab" pluginName="qol-alt-tab" actionLabel="Open switcher" status="linked"
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select} accent="accent" />
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>linked<//>
                    <${Table} columns="8rem 1fr 1fr"><${HotkeyRow} shortcut="Alt+Tab" pluginName="qol-alt-tab" actionLabel="Open switcher" status="linked" accent="accent" /><//>
                    <${StateLabel}>installed<//>
                    <${Table} columns="8rem 1fr 1fr"><${HotkeyRow} shortcut="Super+E" pluginName="qol-launcher" actionLabel="Open launcher" status="installed" accent="accent" /><//>
                    <${StateLabel}>local<//>
                    <${Table} columns="8rem 1fr 1fr"><${HotkeyRow} shortcut="Print" pluginName="qol-screen-recorder" actionLabel="Screenshot" status="local" accent="warning" /><//>
                </div>
            </div>
        <//>
    `;
}

function ShortcutTableShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Shortcut table">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${Table} columns="1fr 5rem 1fr 5rem" onDeselect=${sel.deselect}>
                        <${TableHeader}><${TableCell}>Name<//><${TableCell}>Type<//><${TableCell}>Target<//><${TableCell}>Launcher<//><//>
                        <${ShortcutRow} name="GitHub" type="URL" target="https://github.com" launcher=${true} enabled=${true}
                            selectValue="github" index=${0} selected=${sel.selected('github')} onSelect=${sel.select} />
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>enabled + launcher<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="GitHub" type="URL" target="https://github.com" launcher=${true} enabled=${true} /><//>
                    <${StateLabel}>enabled, no launcher<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="Terminal" type="App" target="com.apple.Terminal" launcher=${false} enabled=${true} /><//>
                    <${StateLabel}>disabled<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="Notes" type="App" target="/usr/bin/notes" launcher=${false} enabled=${false} /><//>
                </div>
            </div>
        <//>
    `;
}

// ---------------------------------------------------------------------------
// Cards
// ---------------------------------------------------------------------------

function StoreCardShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Store cards">
            <div class="catalog-showcase">
                <div class="catalog-try">
                    <${StoreCardGrid} onDeselect=${sel.deselect}>
                        <${StoreCard} name="Alt Tab" version=${{ current: '1.2.0' }} description="Window switcher with live previews"
                            installed=${true} data-plugin-id="plugin-alt-tab" index=${0} selected=${sel.selected(0)} onSelect=${sel.select} />
                    <//>
                </div>
                <div class="catalog-states" inert>
                    <${StateLabel}>installed<//>
                    <${StoreCard} name="Alt Tab" version=${{ current: '1.2.0' }} description="Window switcher" installed=${true} />
                    <${StateLabel}>has update<//>
                    <${StoreCard} name="Launcher" version=${{ from: '2.0.1', to: '2.1.0' }} description="App launcher" installed=${true} hasUpdate=${true} />
                    <${StateLabel}>not installed<//>
                    <${StoreCard} name="Screen Recorder" version=${{ current: '0.3.0' }} description="Record screen" />
                    <${StateLabel}>installing<//>
                    <${StoreCard} name="Window Actions" version=${{ current: '1.0.0' }} description="Minimize, restore" installing=${true} />
                </div>
            </div>
        <//>
    `;
}

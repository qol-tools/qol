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

export function ComponentsCatalog() {
    return html`
        <div class="catalog">
            <${CatalogGroup} title="Display">
                <${ButtonShowcase} />
                <${StatusShowcase} />
                <${SpinnerShowcase} />
                <${EmptyStateShowcase} />
            <//>
            <${CatalogGroup} title="Interactive">
                <${DropdownShowcase} />
                <${ExpanderShowcase} />
                <${ToggleShowcase} />
                <${ModalShowcase} />
                <${DepthDiver} />
            <//>
            <${CatalogGroup} title="Rows" inline=${false}>
                <${DevPluginRowShowcase} />
                <${LogRowShowcase} />
                <${SuppressedRowShowcase} />
                <${BackupRowShowcase} />
            <//>
            <${CatalogGroup} title="Tables" inline=${false}>
                <${HotkeyTableShowcase} />
                <${ShortcutTableShowcase} />
            <//>
            <${CatalogGroup} title="Cards">
                <${StoreCardShowcase} />
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
                <${Button}>Secondary<//>
                <${Button} variant="btn-primary">Primary<//>
                <${Button} variant="btn-ghost">Ghost<//>
                <${Button} variant="btn-danger">Danger<//>
                <${Button} disabled>Disabled<//>
            <//>
            <${CatalogRow} label="Small">
                <${Button} small>Secondary<//>
                <${Button} small variant="btn-primary">Primary<//>
                <${Button} small variant="btn-ghost">Ghost<//>
            <//>
            <${CatalogRow} label="With icons">
                <${Button}><span class="btn-icon">${'\u2193'}</span> Pull<//>
                <${Button}><span class="btn-icon">${'\u2191'}</span> Push<//>
                <${Button} variant="btn-primary"><span class="btn-icon">${'\u26a1'}</span> Connect<//>
                <${Button} variant="btn-ghost">Disconnect<//>
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
    const [toggle3, setToggle3] = useState(true);
    return html`
        <${CatalogSection} title="Toggle">
            <${CatalogRow} label="Basic">
                <${ToggleSwitch} checked=${toggle1} onChange=${setToggle1} label="Enabled" />
                <${ToggleSwitch} checked=${toggle2} onChange=${setToggle2} label="Disabled" />
            <//>
            <${CatalogRow} label="With description">
                <${ToggleSwitch} checked=${toggle3} onChange=${setToggle3}
                    label="Push on change" description="Automatically sync when profile changes" />
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
                <${Button} onActivate=${() => setOpen(true)}>Open test modal<//>
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

function DevPluginRowShowcase() {
    const sel = useListSelection();
    const [linked, setLinked] = useState({ 0: true, 1: false });
    const toggleLink = (i) => setLinked(prev => ({ ...prev, [i]: !prev[i] }));
    const makeActions = (i) => [
        { label: linked[i] ? 'Unlink' : 'Link', run: () => toggleLink(i) },
        { label: 'Toggle logs', run: () => {} },
        { label: 'Edit filters', run: () => {} },
    ];
    return html`
        <${CatalogSection} title="Dev plugin row">
            <div class="plugin-list">
                <${DevPluginRow} name="qol-window-actions" path="~/repos/qol-tools/qol-window-actions"
                    status=${linked[0] ? 'linked' : 'discovered'} pluginId="plugin-window-actions"
                    index=${0} selected=${sel.selected(0)} onSelect=${sel.select}
                    actions=${makeActions(0)}
                    badges=${html`<${Badge} style=${{ background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }}>v1.2.0<//>`}
                    meta=${html`<span style="font-size:var(--fs-xs); color:var(--text-faint)">Built 2m ago</span>`} />
                <${DevPluginRow} name="qol-alt-tab" path="~/repos/qol-tools/qol-alt-tab"
                    status="local" pluginId="plugin-alt-tab"
                    index=${1} selected=${sel.selected(1)} onSelect=${sel.select}
                    actions=${makeActions(1)}
                    badges=${html`<${Badge} style=${{ background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' }}>Local<//>`} />
            </div>
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
                <${LogRow} time="14:32:05" level="error" src="qol-alt-tab" loc="src/main.rs:42" count=${3} severity="warning"
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

const SUPPRESSED_ENTRIES = {
    'qol-alt-tab::hotkey_register_failed': {
        count: 12, last_message: 'Failed to register hotkey: already registered by another process',
        source: 'qol-alt-tab', location: 'src/main.rs:42',
        first_seen: '2026-04-02T14:30:01', last_seen: '2026-04-02T14:32:05',
    },
    'qol-fx::vsync_miss': { count: 47, last_message: 'Animation frame dropped (vsync miss)', source: 'qol-fx', first_seen: '2026-04-02T12:00:00', last_seen: '2026-04-02T14:30:00' },
    'qol-alt-tab::event_loop_stall': { count: 312, source: 'qol-alt-tab', first_seen: '2026-04-01T08:00:00', last_seen: '2026-04-02T14:32:00' },
};

function SuppressedRowShowcase() {
    const sel = useListSelection();
    const [expandedKeys, setExpandedKeys] = useState(new Set());
    const toggle = (key) => setExpandedKeys(prev => {
        const next = new Set(prev);
        if (next.has(key)) next.delete(key); else next.add(key);
        return next;
    });
    const keys = Object.keys(SUPPRESSED_ENTRIES);
    return html`
        <${CatalogSection} title="Card-style">
            <div class="logs-suppressed-list">
                ${keys.map((key, i) => html`
                    <${SuppressedRow} key=${key} sigKey=${key} entry=${SUPPRESSED_ENTRIES[key]}
                        expanded=${expandedKeys.has(key)} index=${i} selected=${sel.selected(i)}
                        onSelect=${sel.select} onToggle=${() => toggle(key)} onUnsuppress=${() => {}} />
                `)}
            </div>
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

function SpinnerShowcase() {
    return html`
        <${CatalogSection} title="Spinner">
            <${CatalogRow} label="States">
                <${RefreshButton} />
                <${RefreshButton} spinning />
            <//>
        <//>
    `;
}

function EmptyStateShowcase() {
    return html`
        <${CatalogSection} title="Empty state">
            <${CatalogRow}>
                <${EmptyState} message="No items found" hint="Try adjusting your filters or adding new items" />
            <//>
        <//>
    `;
}

function HotkeyTableShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Hotkey table">
            <${Table} columns="8rem 1fr 1fr" onDeselect=${sel.deselect}>
                <${TableHeader}>
                    <${TableCell}>Shortcut<//>
                    <${TableCell}>Plugin<//>
                    <${TableCell}>Action<//>
                <//>
                <${HotkeyRow} shortcut="Alt+Tab" pluginName="qol-alt-tab" actionLabel="Open switcher" status="linked"
                    index=${0} selected=${sel.selected(0)} onSelect=${sel.select} accent="accent" />
                <${HotkeyRow} shortcut="Super+E" pluginName="qol-launcher" actionLabel="Open launcher" status="installed"
                    index=${1} selected=${sel.selected(1)} onSelect=${sel.select} accent="accent" />
                <${HotkeyRow} shortcut="Print" pluginName="qol-screen-recorder" actionLabel="Screenshot" status="local"
                    index=${2} selected=${sel.selected(2)} onSelect=${sel.select} accent="warning" />
            <//>
        <//>
    `;
}

function ShortcutTableShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Shortcut table">
            <${Table} columns="1fr 5rem 1fr 5rem" onDeselect=${sel.deselect}>
                <${TableHeader}>
                    <${TableCell}>Name<//>
                    <${TableCell}>Type<//>
                    <${TableCell}>Target<//>
                    <${TableCell}>Launcher<//>
                <//>
                <${ShortcutRow} name="GitHub" type="URL" target="https://github.com" launcher=${true} enabled=${true}
                    selectValue="github" index=${0} selected=${sel.selected('github')} onSelect=${sel.select} />
                <${ShortcutRow} name="Terminal" type="App" target="com.apple.Terminal" launcher=${true} enabled=${true}
                    selectValue="terminal" index=${1} selected=${sel.selected('terminal')} onSelect=${sel.select} />
                <${ShortcutRow} name="Notes" type="App" target="/usr/bin/notes" launcher=${false} enabled=${false}
                    selectValue="notes" index=${2} selected=${sel.selected('notes')} onSelect=${sel.select} />
            <//>
        <//>
    `;
}

function StoreCardShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Store cards">
            <${StoreCardGrid} onDeselect=${sel.deselect}>
                <${StoreCard} name="Alt Tab" version=${{ current: '1.2.0' }} description="Window switcher with live previews"
                    installed=${true} data-plugin-id="plugin-alt-tab" index=${0} selected=${sel.selected(0)} onSelect=${sel.select} />
                <${StoreCard} name="Launcher" version=${{ from: '2.0.1', to: '2.1.0' }} description="App launcher with fuzzy search"
                    installed=${true} hasUpdate=${true} data-plugin-id="plugin-launcher" index=${1} selected=${sel.selected(1)} onSelect=${sel.select} />
                <${StoreCard} name="Screen Recorder" version=${{ current: '0.3.0' }} description="Record screen, window, or region"
                    data-plugin-id="plugin-screen-recorder" index=${2} selected=${sel.selected(2)} onSelect=${sel.select} />
                <${StoreCard} name="Window Actions" version=${{ current: '1.0.0' }} description="Minimize, restore, move between monitors"
                    installing=${true} data-plugin-id="plugin-window-actions" index=${3} selected=${sel.selected(3)} onSelect=${sel.select} />
            <//>
        <//>
    `;
}

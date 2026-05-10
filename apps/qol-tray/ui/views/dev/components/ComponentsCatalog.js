import { html } from '../../../lib/html.js';
import { useCallback, useRef, useState } from 'preact/hooks';
import { SurfaceContainer } from '../../../lib/components/SurfaceContainer.js';
import { directSurfaces } from '../../../lib/surface-traits.js';
import { ToggleSwitch } from '../../../lib/components/ToggleSwitch.js';
import { CustomSelect } from '../../../lib/components/CustomSelect.js';
import { Expander, ExpanderTrigger, ExpanderBody } from '../../../lib/components/Expander.js';
import { Badge, HealthDot, Alert } from '../../../lib/components/StatusIndicators.js';
import { ListGroup } from '../../../lib/components/ListRow.js';
import { Table, TableHeader, TableCell } from '../../../lib/components/TableRow.js';
import { Surface } from '../../../lib/components/Surface.js';
import { Button, RefreshButton } from '../../../lib/components/Button.js';
import { EmptyState } from '../../../lib/components/EmptyState.js';
import { useListSelection } from '../../../lib/hooks/useListSelection.js';
import { LogRow } from '../../../components/domain-rows/LogRow.js';
import { galleryLogRowSlot } from '../gallery-log-row-detail-subpage.js';
import { SuppressedRow } from '../../../components/domain-rows/SuppressedRow.js';
import { BackupRow } from '../../../components/domain-rows/BackupRow.js';
import { galleryBackupRowSlot } from '../gallery-backup-row-detail-subpage.js';
import { toast } from '../../../lib/toast.js';
import { HotkeyRow } from '../../../components/domain-rows/HotkeyRow.js';
import { useGalleryHotkeyEditorController } from '../gallery-hotkey-editor-subpage.js';
import { ShortcutRow } from '../../../components/domain-rows/ShortcutRow.js';
import { useGalleryShortcutEditorController } from '../gallery-shortcut-editor-subpage.js';
import { DevPluginRow } from '../../../components/domain-rows/DevPluginRow.js';
import { StoreCard, StoreCardGrid } from '../../../components/domain-rows/StoreCard.js';
import { KeyLegend } from '../../../lib/components/KeyLegend.js';

const SHOWCASES = {
    buttons: ButtonShowcase,
    status: StatusShowcase,
    spinner: SpinnerShowcase,
    'empty-state': EmptyStateShowcase,
    dropdown: DropdownShowcase,
    expander: ExpanderShowcase,
    toggle: ToggleShowcase,
    'depth-diver': DepthDiver,
    'key-legend': KeyLegendShowcase,
    'dev-plugin-row': DevPluginRowShowcase,
    'log-row': LogRowShowcase,
    'suppressed-row': SuppressedRowShowcase,
    'backup-row': BackupRowShowcase,
    'hotkey-row': HotkeyTableShowcase,
    'shortcut-row': ShortcutTableShowcase,
    'store-card': StoreCardShowcase,
};

export const SHOWCASE_KEYS = Object.keys(SHOWCASES);

export function ComponentsCatalog({ activeId }) {
    const Showcase = SHOWCASES[activeId];
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

function Interactive({ inert, children }) {
    return html`
        <div class="catalog-column">
            <div class="catalog-column-label">Interactive</div>
            <div class="catalog-try" inert=${inert || null}>${children}</div>
        </div>
    `;
}

function States({ className, children }) {
    const cls = ['catalog-states', className].filter(Boolean).join(' ');
    return html`
        <div class="catalog-column">
            <div class="catalog-column-label">States</div>
            <div class=${cls} inert>${children}</div>
        </div>
    `;
}

function StateLabel({ children }) {
    return html`<div class="catalog-state-label">${children}</div>`;
}

function MockControls({ actions }) {
    if (!actions?.length) return null;
    return html`
        <div class="catalog-mock-controls">
            ${actions.map(a => html`<button key=${a.label} class="btn btn-sm btn-ghost" onClick=${a.run}>${a.label}</button>`)}
        </div>
    `;
}

function ButtonShowcase() {
    return html`
        <${CatalogSection} title="Buttons">
            <div class="catalog-showcase">
                <${Interactive}>
                    <div><${Button} variant="btn-primary" onActivate=${() => {}}>Interactive<//></div>
                <//>
                <${States}>
                    <${StateLabel}>variants<//>
                    <div class="catalog-state-inline">
                        <${Button}>Secondary<//> <${Button} variant="btn-primary">Primary<//> <${Button} variant="btn-ghost">Ghost<//> <${Button} variant="btn-danger">Danger<//> <${Button} disabled>Disabled<//>
                    </div>
                    <${StateLabel}>small<//>
                    <div class="catalog-state-inline">
                        <${Button} small>Secondary<//> <${Button} small variant="btn-primary">Primary<//> <${Button} small variant="btn-ghost">Ghost<//>
                    </div>
                <//>
            </div>
        <//>
    `;
}

function StatusShowcase() {
    return html`
        <${CatalogSection} title="Status indicators">
            <div class="catalog-showcase">
                <${Interactive} inert>
                    <div class="catalog-state-inline">
                        <${Badge}>3<//>
                        <${Badge} style=${{ background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }}>OK<//>
                        <${Badge} style=${{ background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' }}>Review<//>
                    </div>
                <//>
                <${States}>
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
                <//>
            </div>
        <//>
    `;
}

function SpinnerShowcase() {
    return html`
        <${CatalogSection} title="Spinner">
            <div class="catalog-showcase">
                <${Interactive} inert><${RefreshButton} /><//>
                <${States}>
                    <${StateLabel}>idle<//>
                    <${RefreshButton} />
                    <${StateLabel}>spinning<//>
                    <${RefreshButton} spinning />
                <//>
            </div>
        <//>
    `;
}

function EmptyStateShowcase() {
    return html`
        <${CatalogSection} title="Empty state">
            <div inert>
                <${EmptyState} message="No items found" hint="Try adjusting your filters or adding new items" />
            </div>
        <//>
    `;
}

function DropdownShowcase() {
    const options = ['github', 'folder', 'local'];
    const labels = { github: 'GitHub', folder: 'Folder', local: 'Local' };
    const [value, setValue] = useState('github');
    return html`
        <${CatalogSection} title="Dropdown">
            <div class="catalog-showcase">
                <${Interactive}><${CustomSelect} value=${value} options=${options} labels=${labels} onChange=${setValue} /><//>
                <${States}>
                    <${StateLabel}>closed<//>
                    <${CustomSelect} value="github" options=${options} labels=${labels} onChange=${() => {}} />
                    <${StateLabel}>compact<//>
                    <${CustomSelect} value="folder" options=${options} labels=${labels} onChange=${() => {}} compact=${true} />
                    <${StateLabel}>open<//>
                    <div class="custom-select" style="position:relative">
                        <button class="btn btn-dropdown custom-select-trigger" type="button" aria-expanded="true">
                            <span class="custom-select-value">GitHub</span>
                            <span class="custom-select-arrow">${'\u25BE'}</span>
                        </button>
                        <div class="custom-select-popover" style="position:relative; display:block;">
                            <div class="custom-select-list" style="position:relative;">
                                <div class="custom-select-option selected highlighted">GitHub</div>
                                <div class="custom-select-option">Folder</div>
                                <div class="custom-select-option">Local</div>
                            </div>
                        </div>
                    </div>
                <//>
            </div>
        <//>
    `;
}

function ExpanderShowcase() {
    const [open, setOpen] = useState(false);
    return html`
        <${CatalogSection} title="Expander">
            <div class="catalog-showcase">
                <${Interactive}>
                    <${Expander} open=${open} onToggle=${() => setOpen(!open)}>
                        <${ExpanderTrigger}>${open ? 'Expanded' : 'Collapsed'}<//>
                        <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Content here.</span><//>
                    <//>
                <//>
                <${States}>
                    <${StateLabel}>collapsed<//>
                    <${Expander} open=${false} onToggle=${() => {}}>
                        <${ExpanderTrigger}>Section A<//>
                    <//>
                    <${StateLabel}>expanded<//>
                    <${Expander} open=${true} onToggle=${() => {}}>
                        <${ExpanderTrigger}>Section B<//>
                        <${ExpanderBody}><span style="color:var(--text-muted); font-size:var(--fs-sm);">Visible content.</span><//>
                    <//>
                <//>
            </div>
        <//>
    `;
}

function ToggleShowcase() {
    const [on, setOn] = useState(true);
    return html`
        <${CatalogSection} title="Toggle">
            <div class="catalog-showcase">
                <${Interactive}>
                    <${ToggleSwitch} checked=${on} onChange=${setOn} label="Push on change" description="Automatically sync when profile changes" />
                <//>
                <${States}>
                    <${StateLabel}>on<//>
                    <${ToggleSwitch} checked=${true} onChange=${() => {}} label="Enabled" />
                    <${StateLabel}>off<//>
                    <${ToggleSwitch} checked=${false} onChange=${() => {}} label="Disabled" />
                <//>
            </div>
        <//>
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

function KeyLegendShowcase() {
    return html`
        <${CatalogSection} title="Key legend">
            <div class="catalog-showcase">
                <${Interactive} inert>
                    <${KeyLegend} bindings=${[
                        { action: 'add', key: 'a', label: 'add' },
                        { action: 'edit', key: 'Enter', label: 'edit' },
                        { action: 'delete', key: 'Delete', label: 'delete' },
                        { action: 'run', key: 'r', label: 'run' },
                    ]} />
                <//>
                <${States}>
                    <${StateLabel}>minimal<//>
                    <${KeyLegend} bindings=${[{ action: 'add', key: 'a', label: 'add' }]} />
                    <${StateLabel}>with modifier<//>
                    <${KeyLegend} bindings=${[
                        { action: 'save', key: 'Ctrl+Enter', label: 'save' },
                        { action: 'close', key: 'Esc', label: 'close' },
                    ]} />
                    <${StateLabel}>empty (renders nothing)<//>
                    <${KeyLegend} bindings=${[]} />
                <//>
            </div>
        <//>
    `;
}

function DevPluginRowShowcase() {
    const sel = useListSelection();
    const [linked, setLinked] = useState(false);
    const [needsRebuild, setNeedsRebuild] = useState(false);
    const [busyType, setBusyType] = useState(null);
    const builtRef = useRef(false);
    const doLink = useCallback(() => {
        setBusyType('linking');
        setTimeout(() => { setLinked(true); setNeedsRebuild(!builtRef.current); setBusyType(null); }, 600);
    }, []);
    const doRebuild = useCallback(() => {
        setBusyType('building');
        setTimeout(() => { builtRef.current = true; setNeedsRebuild(false); setBusyType(null); }, 600);
    }, []);
    const doUnlink = useCallback(() => { setLinked(false); setNeedsRebuild(false); }, []);
    const doReset = useCallback(() => { setLinked(false); setNeedsRebuild(false); builtRef.current = false; }, []);
    const rebuildActive = linked && needsRebuild;
    const busy = busyType !== null;
    const rebuildIcon = (cls) => html`<img class="plugin-action-rebuild-icon ${cls}" src="assets/qol-tray.png?v=1" alt="" aria-hidden="true" />`;
    const linkLabel = rebuildActive ? 'Rebuild' : linked ? 'Unlink' : 'Link';
    const linkRun = rebuildActive ? doRebuild : linked ? doUnlink : doLink;
    const actions = busy ? [] : [
        { label: linkLabel, run: linkRun },
        ...(linked ? [{ label: 'Mute Logs', run: () => {} }, { label: 'Edit Filters', run: () => {} }] : []),
    ];
    const icon = busy
        ? html`<span class="refresh-btn spinning" aria-hidden="true"></span>`
        : rebuildIcon(rebuildActive ? 'has-rebuild' : 'rebuild-idle');
    const status = linked ? 'linked' : 'local';
    const badgeText = busy ? (busyType === 'linking' ? 'Linking...' : 'Building...') : rebuildActive ? 'Needs rebuild' : linked ? 'Linked' : 'Local';
    const badgeColor = (!busy && linked && !needsRebuild) ? 'success' : 'warning';
    const badgeStyle = badgeColor === 'success'
        ? { background: 'rgba(var(--success-rgb),0.14)', borderColor: 'rgba(var(--success-rgb),0.26)' }
        : { background: 'rgba(var(--warning-rgb),0.16)', borderColor: 'rgba(var(--warning-rgb),0.3)' };
    const mockActions = [];
    if (linked && !needsRebuild && !busy) mockActions.push({ label: 'Dirty source', run: () => setNeedsRebuild(true) });
    if ((linked || builtRef.current) && !busy) mockActions.push({ label: 'Reset', run: doReset });

    return html`
        <${CatalogSection} title="Dev plugin row">
            <div class="catalog-showcase">
                <${Interactive}>
                    <div class="plugin-list">
                        <${DevPluginRow} name="qol-window-actions" path="~/repos/qol-tools/qol-window-actions"
                            status=${status}
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select}
                            actions=${actions} actionIcon=${icon}
                            className=${busy ? 'is-linking' : undefined}
                            badges=${html`<${Badge} style=${badgeStyle}>${badgeText}<//>`}
                            meta=${html`<span style="font-size:var(--fs-xs); color:var(--text-faint)">${busy ? 'Compiling...' : rebuildActive ? 'Source changed • fp a3b2c1d0' : linked ? 'fp a3b2c1d0 • Built 2m ago' : ''}</span>`} />
                    </div>
                    <${MockControls} actions=${mockActions} />
                <//>
                <${States}>
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
                <//>
            </div>
        <//>
    `;
}

const SAMPLE_LOG_ENTRIES = [
    { time: '14:32:01', level: 'startup', src: 'qol-window-actions', loc: 'src/lib.rs:18',
      msg: 'Plugin initialized successfully' },
    { time: '14:32:05', level: 'error', src: 'qol-alt-tab', loc: 'src/main.rs:42',
      count: 3, severity: 'warning', msg: 'Failed to register hotkey: already registered' },
    { time: '14:32:08', level: 'suppressed', src: 'qol-fx',
      msg: 'Animation frame dropped (vsync miss)' },
];

function LogRowShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Log row">
            <${ListGroup} onDeselect=${sel.deselect}>
                ${SAMPLE_LOG_ENTRIES.map((entry, i) => html`
                    <${LogRow} key=${i} ...${entry}
                        index=${i} selected=${sel.selected(i)} onSelect=${sel.select}
                        onActivate=${() => galleryLogRowSlot.set({ entry })} />
                `)}
            <//>
        <//>
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
                <${Interactive}>
                    <${ListGroup} className="logs-suppressed-list" onDeselect=${sel.deselect}>
                        <${SuppressedRow} sigKey="qol-alt-tab::hotkey_register_failed" entry=${entry}
                            expanded=${expanded} index=${0} selected=${sel.selected(0)}
                            onSelect=${sel.select} onToggle=${() => setExpanded(e => !e)} onUnsuppress=${() => {}} />
                    <//>
                <//>
                <${States} className="logs-suppressed-list">
                    <${StateLabel}>collapsed<//>
                    <${SuppressedRow} sigKey="qol-fx::vsync_miss" entry=${{ count: 47, last_message: 'Animation frame dropped' }} expanded=${false} onToggle=${() => {}} />
                    <${StateLabel}>expanded<//>
                    <${SuppressedRow} sigKey="qol-alt-tab::event_loop_stall" entry=${{ count: 312, source: 'qol-alt-tab', first_seen: '2026-04-01T08:00:00', last_seen: '2026-04-02T14:32:00' }} expanded=${true} onToggle=${() => {}} />
                <//>
            </div>
        <//>
    `;
}

const SAMPLE_BACKUP_TOML = `# qol-tray profile backup
schema_version = 1

[hotkeys]
toggle-alt-tab = "Alt+Tab"
launch-terminal = "Ctrl+Alt+T"

[plugins.alt-tab]
enabled = true
preview_size = 220

[plugins.lights]
enabled = false
`;

const SAMPLE_BACKUP_ENTRIES = [
    { time: '2026-04-02 14:30', fileName: 'profile-backup-2026-04-02T143001.toml',
      size: '2.4 KB', review: true, content: SAMPLE_BACKUP_TOML },
    { time: '2026-04-02 14:30', fileName: 'profile-backup.toml',
      size: '2.4 KB', review: true, content: SAMPLE_BACKUP_TOML },
    { time: '2026-04-01 09:15', fileName: 'profile-backup.toml',
      size: '1.8 KB', content: SAMPLE_BACKUP_TOML },
];

function BackupRowShowcase() {
    const sel = useListSelection();
    return html`
        <${CatalogSection} title="Backup row">
            <${ListGroup} onDeselect=${sel.deselect}>
                ${SAMPLE_BACKUP_ENTRIES.map((entry, i) => html`
                    <${BackupRow} key=${i} ...${entry}
                        index=${i} selected=${sel.selected(i)} onSelect=${sel.select}
                        data-dive-target="profile-backup-detail"
                        data-secondary-label="Open in editor"
                        onActivate=${() => galleryBackupRowSlot.set({
                            preview: { file_name: entry.fileName, content: entry.content },
                            incident: entry.review ? { backup_file: entry.fileName } : null,
                            onAcknowledge: null,
                        })}
                        onSecondaryActivate=${() => toast('info', 'Open in editor (gallery sandbox)')} />
                `)}
            <//>
        <//>
    `;
}

function HotkeyTableShowcase() {
    const sel = useListSelection();
    const editor = useGalleryHotkeyEditorController();
    const sample = { id: 'gallery-1', plugin_id: 'qol-alt-tab', action: 'open-switcher', key: 'Alt+Tab' };
    return html`
        <${CatalogSection} title="Hotkey table">
            <div class="catalog-showcase">
                <${Interactive}>
                    <${Table} columns="8rem 1fr 1fr" onDeselect=${sel.deselect}>
                        <${TableHeader}><${TableCell}>Shortcut<//><${TableCell}>Plugin<//><${TableCell}>Action<//><//>
                        <${HotkeyRow} shortcut="Alt+Tab" pluginName="qol-alt-tab" actionLabel="Open switcher"
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select} accent="accent"
                            data-dive-target="dev-gallery-hotkey-row-editor"
                            onActivate=${() => editor.open(sample)} />
                    <//>
                <//>
                <${States}>
                    <${StateLabel}>enabled<//>
                    <${Table} columns="8rem 1fr 1fr"><${HotkeyRow} shortcut="Alt+Tab" pluginName="qol-alt-tab" actionLabel="Open switcher" accent="accent" /><//>
                    <${StateLabel}>disabled<//>
                    <${Table} columns="8rem 1fr 1fr"><${HotkeyRow} shortcut="Super+E" pluginName="qol-launcher" actionLabel="Open launcher" className="disabled" /><//>
                <//>
            </div>
        <//>
    `;
}

function ShortcutTableShowcase() {
    const sel = useListSelection();
    const editor = useGalleryShortcutEditorController();
    const sample = {
        id: 'github', name: 'GitHub', enabled: true, export_to_launcher: true,
        action: { type: 'open_url', url: 'https://github.com' },
    };
    return html`
        <${CatalogSection} title="Shortcut table">
            <div class="catalog-showcase">
                <${Interactive}>
                    <${Table} columns="1fr 5rem 1fr 5rem" onDeselect=${sel.deselect}>
                        <${TableHeader}><${TableCell}>Name<//><${TableCell}>Type<//><${TableCell}>Target<//><${TableCell}>Launcher<//><//>
                        <${ShortcutRow} name="GitHub" type="URL" target="https://github.com" launcher=${true} enabled=${true}
                            selectValue="github" index=${0} selected=${sel.selected('github')} onSelect=${sel.select}
                            data-dive-target="dev-gallery-shortcut-row-editor"
                            onActivate=${() => editor.open(sample)} />
                    <//>
                <//>
                <${States}>
                    <${StateLabel}>enabled + launcher<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="GitHub" type="URL" target="https://github.com" launcher=${true} enabled=${true} /><//>
                    <${StateLabel}>enabled, no launcher<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="Terminal" type="App" target="com.apple.Terminal" launcher=${false} enabled=${true} /><//>
                    <${StateLabel}>disabled<//>
                    <${Table} columns="1fr 5rem 1fr 5rem"><${ShortcutRow} name="Notes" type="App" target="/usr/bin/notes" launcher=${false} enabled=${false} /><//>
                <//>
            </div>
        <//>
    `;
}

function StoreCardShowcase() {
    const sel = useListSelection();
    const [installed, setInstalled] = useState(false);
    const [installing, setInstalling] = useState(false);
    const [hasUpdate, setHasUpdate] = useState(false);
    const [ver, setVer] = useState('1.2.0');
    const doInstall = useCallback(() => {
        setInstalling(true);
        setTimeout(() => { setInstalling(false); setInstalled(true); }, 800);
    }, []);
    const doUninstall = useCallback(() => { setInstalled(false); setHasUpdate(false); }, []);
    const doUpdate = useCallback(() => {
        setInstalling(true);
        const next = ver.replace(/(\d+)$/, m => String(Number(m) + 1));
        setTimeout(() => { setInstalling(false); setHasUpdate(false); setVer(next); }, 800);
    }, [ver]);
    const mockUpdate = useCallback(() => setHasUpdate(true), []);
    const onActivate = installing ? undefined
        : !installed ? doInstall
        : hasUpdate ? doUpdate
        : undefined;
    const nextVer = ver.replace(/(\d+)$/, m => String(Number(m) + 1));
    const version = hasUpdate ? { from: ver, to: nextVer } : { current: ver };
    return html`
        <${CatalogSection} title="Store cards">
            <div class="catalog-showcase">
                <${Interactive}>
                    <${StoreCardGrid} onDeselect=${sel.deselect}>
                        <${StoreCard} name="Alt Tab" version=${version} description="Window switcher with live previews"
                            installed=${installed} installing=${installing} hasUpdate=${hasUpdate}
                            index=${0} selected=${sel.selected(0)} onSelect=${sel.select} onActivate=${onActivate} />
                    <//>
                    <${MockControls} actions=${[
                        ...(installed && !hasUpdate && !installing ? [{ label: 'Mock update', run: mockUpdate }] : []),
                        ...(installed && !installing ? [{ label: 'Uninstall', run: doUninstall }] : []),
                        ...(installed || installing ? [{ label: 'Reset', run: () => { setInstalled(false); setInstalling(false); setHasUpdate(false); setVer('1.2.0'); } }] : []),
                    ]} />
                <//>
                <${States}>
                    <${StateLabel}>not installed<//>
                    <${StoreCardGrid}><${StoreCard} name="Screen Recorder" version=${{ current: '0.3.0' }} description="Record screen" /><//>
                    <${StateLabel}>installing<//>
                    <${StoreCardGrid}><${StoreCard} name="Window Actions" version=${{ current: '1.0.0' }} description="Minimize, restore" installing=${true} /><//>
                    <${StateLabel}>installed<//>
                    <${StoreCardGrid}><${StoreCard} name="Alt Tab" version=${{ current: '1.2.0' }} description="Window switcher" installed=${true} /><//>
                    <${StateLabel}>update available<//>
                    <${StoreCardGrid}><${StoreCard} name="Launcher" version=${{ from: '2.0.1', to: '2.1.0' }} description="App launcher" installed=${true} hasUpdate=${true} /><//>
                <//>
            </div>
        <//>
    `;
}

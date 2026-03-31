import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { useGridNav } from '../../hooks/useGridNav.js';
import { providerFieldSurfaceId } from './form.js';
import { connectActionLabel } from './summary.js';

export const PROFILE_SURFACE_SELECTOR = '#profile-page [data-selected-surface]';

export function surfaceProps(surface, selectedIndex, setSelectedIndex) {
    if (!surface) return null;
    const selected = surface.index === selectedIndex;
    return {
        'data-selected-surface': '',
        'data-selected': selected ? 'true' : 'false',
        'data-index': String(surface.index),
        onMouseDown: () => setSelectedIndex(surface.index),
        onFocus: () => setSelectedIndex(surface.index),
    };
}

export function useSurfaceNav({
    advancedProviderFields,
    backups,
    basicProviderFields,
    configured,
    form,
    handleAcknowledge,
    handleConnect,
    handleDisconnect,
    handleExport,
    handleImport,
    handleOpenBackups,
    handlePreviewBackup,
    handlePull,
    handlePush,
    importBusy,
    incident,
    syncBusy,
    updateForm,
}) {
    const [selectedIndex, setSelectedIndex] = useState(0);
    const didInitSelectionRef = useRef(false);
    const selectedIndexRef = useRef(selectedIndex);
    selectedIndexRef.current = selectedIndex;

    const surfaces = useMemo(() => {
        const next = [];
        const add = (id, kind, extra = {}) => {
            next.push({ id, kind, index: next.length, ...extra });
        };

        // Connected: action row surfaces
        if (configured) {
            add('pull', 'action', { label: 'Pull Now', run: syncBusy ? null : handlePull });
            add('push', 'action', { label: 'Push Now', run: syncBusy ? null : handlePush });
        }
        if (incident) {
            add('acknowledge', 'action', {
                label: 'Acknowledge',
                variant: 'btn-primary',
                run: syncBusy ? null : handleAcknowledge,
            });
        }
        if (configured) {
            add('disconnect', 'action', { label: 'Disconnect', variant: 'btn-ghost', run: syncBusy ? null : handleDisconnect });
        }
        // Not connected: connect button
        if (!configured) {
            add('connect', 'action', {
                label: connectActionLabel(configured, form.provider),
                variant: 'btn-primary',
                run: syncBusy ? null : handleConnect,
            });
        }
        // Settings expander toggle
        add('settings', 'toggle', { run: null });
        // Settings expander surfaces
        add('provider', 'field');
        basicProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        advancedProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        // Backups section
        add('export', 'action', {
            label: 'Export',
            variant: 'btn-ghost',
            run: importBusy ? null : handleExport,
        });
        add('import', 'action', {
            label: 'Import',
            variant: 'btn-ghost',
            run: importBusy ? null : handleImport,
        });
        add('open-backups', 'action', { label: 'Open Folder', run: handleOpenBackups });
        backups.forEach(backup => {
            add(`backup:${backup.file_name}`, 'action', {
                label: backup.file_name,
                run: () => handlePreviewBackup(backup.file_name),
            });
        });

        return next;
    }, [
        advancedProviderFields,
        backups,
        basicProviderFields,
        configured,
        form.pull_on_launch,
        form.push_on_change,
        handleAcknowledge,
        handleConnect,
        handleDisconnect,
        handleExport,
        handleImport,
        handleOpenBackups,
        handlePreviewBackup,
        handlePull,
        handlePush,
        importBusy,
        incident,
        syncBusy,
        updateForm,
    ]);

    useEffect(() => {
        setSelectedIndex(index => Math.min(index, Math.max(0, surfaces.length - 1)));
    }, [surfaces.length]);

    const surfaceById = useMemo(
        () => new Map(surfaces.map(surface => [surface.id, surface])),
        [surfaces]
    );

    useEffect(() => {
        if (didInitSelectionRef.current) return;
        if (surfaces.length === 0) return;
        didInitSelectionRef.current = true;
        setSelectedIndex(0);
    }, [surfaces.length]);

    const navigateInGrid = useGridNav(PROFILE_SURFACE_SELECTOR, selectedIndexRef, setSelectedIndex);

    const activateSelected = useCallback(() => {
        const surface = surfaces[selectedIndexRef.current];
        if (!surface) {
            return;
        }
        if (surface.kind === 'action' || surface.kind === 'toggle') {
            if (surface.run) { surface.run(); return; }
            const el = surfaceElement(surface.index);
            if (el) el.click();
            return;
        }
        enterSurfaceEditor(surface.index);
    }, [selectedIndexRef, surfaces]);

    const focusSelectedSurface = useCallback(() => {
        focusSurface(selectedIndexRef.current);
    }, [selectedIndexRef]);

    const commands = useMemo(() => {
        const next = [
            { id: 'profile:export', label: 'Export profile', run: handleExport },
            { id: 'profile:import', label: 'Import profile', run: handleImport },
            { id: 'profile:backups', label: 'Open backups folder', run: handleOpenBackups },
        ];
        if (configured) {
            next.unshift(
                { id: 'profile:push', label: 'Push cloud sync', run: handlePush },
                { id: 'profile:pull', label: 'Pull cloud sync', run: handlePull },
            );
        }
        if (incident) {
            next.unshift({ id: 'profile:acknowledge', label: 'Acknowledge sync review', run: handleAcknowledge });
        }
        next.unshift({
            id: 'profile:connect',
            label: connectActionLabel(configured, form.provider),
            run: handleConnect,
        });
        return next;
    }, [
        configured,
        handleAcknowledge,
        handleConnect,
        handleExport,
        handleImport,
        handleOpenBackups,
        handlePull,
        handlePush,
        incident,
        form.provider,
    ]);

    return {
        activateSelected,
        commands,
        focusSelectedSurface,
        navigateInGrid,
        selectedIndex,
        selectedIndexRef,
        setSelectedIndex,
        surfaceById,
    };
}

function enterSurfaceEditor(index) {
    const container = surfaceElement(index);
    if (!(container instanceof HTMLElement)) return;
    const trigger = container.querySelector('.custom-select-trigger');
    if (trigger instanceof HTMLButtonElement) { trigger.focus(); trigger.click(); return; }
    const input = container.querySelector('[data-profile-editable]');
    if (input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement) { input.focus(); input.select?.(); return; }
    const link = container.querySelector('a[href]');
    if (link instanceof HTMLAnchorElement) { link.click(); return; }
    const button = container.querySelector('button');
    if (button instanceof HTMLButtonElement) { button.click(); return; }
}

function focusSurface(index) {
    const element = surfaceElement(index);
    if (!(element instanceof HTMLElement)) {
        return;
    }
    element.focus();
}

function surfaceElement(index) {
    return document.querySelector(`${PROFILE_SURFACE_SELECTOR}[data-index="${index}"]`);
}

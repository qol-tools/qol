import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { useGridNav } from '../../hooks/useGridNav.js';
import { providerFieldSurfaceId } from './form.js';
import { connectActionLabel } from './summary.js';

export const PROFILE_SURFACE_SELECTOR = '#profile-page [data-selected-surface]';

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

        add('export', 'action', {
            label: 'Export Profile',
            variant: 'btn-ghost',
            run: importBusy ? null : handleExport,
        });
        add('import', 'action', {
            label: 'Import Profile',
            variant: 'btn-ghost',
            run: importBusy ? null : handleImport,
        });
        add('provider', 'field');
        basicProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        advancedProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        add('pull-on-launch', 'toggle', {
            run: () => updateForm('pull_on_launch', !form.pull_on_launch),
        });
        add('push-on-change', 'toggle', {
            run: () => updateForm('push_on_change', !form.push_on_change),
        });
        if (configured) {
            add('disconnect', 'action', { label: 'Disconnect', run: syncBusy ? null : handleDisconnect });
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
        add('connect', 'action', {
            label: connectActionLabel(configured, form.provider),
            variant: 'btn-primary',
            run: syncBusy ? null : handleConnect,
        });
        add('open-backups', 'action', { label: 'Open Backups Folder', run: handleOpenBackups });
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
        if (didInitSelectionRef.current) {
            return;
        }
        const firstField = basicProviderFields[0];
        const initial = surfaceById.get('provider') || (firstField ? surfaceById.get(providerFieldSurfaceId(firstField.key)) : null);
        if (!initial) {
            return;
        }
        didInitSelectionRef.current = true;
        setSelectedIndex(initial.index);
    }, [basicProviderFields, surfaceById]);

    const navigateInGrid = useGridNav(PROFILE_SURFACE_SELECTOR, selectedIndexRef, setSelectedIndex);

    const activateSelected = useCallback(() => {
        const surface = surfaces[selectedIndexRef.current];
        if (!surface) {
            return;
        }
        if (surface.kind === 'action' || surface.kind === 'toggle') {
            surface.run?.();
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
    if (!(container instanceof HTMLElement)) {
        return;
    }
    const trigger = container.querySelector('.custom-select-trigger');
    if (trigger instanceof HTMLButtonElement) {
        trigger.focus();
        trigger.click();
        return;
    }
    const input = container.querySelector('[data-profile-editable]');
    if (input instanceof HTMLInputElement || input instanceof HTMLTextAreaElement) {
        input.focus();
        input.select?.();
    }
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

import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { providerFieldSurfaceId } from './form.js';
import { busyActionLabel, connectActionLabel } from './summary.js';

export function useSurfaceNav({
    advancedProviderFields,
    authPrompt,
    backups,
    basicProviderFields,
    busyAction,
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

    const surfaces = useMemo(() => {
        const next = [];
        const add = (id, kind, extra = {}) => {
            next.push({ id, kind, index: next.length, ...extra });
        };
        const actionBusy = (id) => busyAction === id;

        if (configured) {
            add('pull', 'action', { label: busyActionLabel('pull', actionBusy('pull')), run: syncBusy ? null : handlePull, busy: actionBusy('pull') });
            add('push', 'action', { label: busyActionLabel('push', actionBusy('push')), run: syncBusy ? null : handlePush, busy: actionBusy('push') });
        }
        if (incident) {
            add('acknowledge', 'action', { label: busyActionLabel('acknowledge', actionBusy('acknowledge')), variant: 'btn-primary', run: syncBusy ? null : handleAcknowledge, busy: actionBusy('acknowledge') });
        }
        if (configured) {
            add('disconnect', 'action', { label: busyActionLabel('disconnect', actionBusy('disconnect')), variant: 'btn-ghost', run: syncBusy ? null : handleDisconnect, busy: actionBusy('disconnect') });
        }
        if (!configured) {
            const connectBusy = actionBusy('connect') && !authPrompt;
            add('connect', 'action', {
                label: connectBusy ? busyActionLabel('connect', true) : connectActionLabel(configured, form.provider),
                variant: 'btn-primary',
                run: syncBusy ? null : handleConnect,
                busy: connectBusy,
            });
        }
        add('settings', 'toggle', { run: null });
        add('provider', 'field');
        basicProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        advancedProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        add('export', 'action', { label: busyActionLabel('export', actionBusy('export')), variant: 'btn-ghost', run: importBusy ? null : handleExport, busy: actionBusy('export') });
        add('import', 'action', { label: busyActionLabel('import', actionBusy('import')), variant: 'btn-ghost', run: importBusy ? null : handleImport, busy: actionBusy('import') });
        add('open-backups', 'action', { label: 'Open Folder', run: handleOpenBackups });
        backups.forEach(backup => {
            add(`backup:${backup.file_name}`, 'action', { label: backup.file_name, run: () => handlePreviewBackup(backup.file_name) });
        });

        return next;
    }, [
        advancedProviderFields, authPrompt, backups, basicProviderFields, busyAction, configured,
        form.pull_on_launch, form.push_on_change, form.provider,
        handleAcknowledge, handleConnect, handleDisconnect, handleExport,
        handleImport, handleOpenBackups, handlePreviewBackup, handlePull, handlePush,
        importBusy, incident, syncBusy, updateForm,
    ]);

    useEffect(() => {
        setSelectedIndex(index => Math.min(index, Math.max(0, surfaces.length - 1)));
    }, [surfaces.length]);

    const surfaceById = useMemo(
        () => new Map(surfaces.map(surface => [surface.id, surface])),
        [surfaces]
    );

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
        next.unshift({ id: 'profile:connect', label: connectActionLabel(configured, form.provider), run: handleConnect });
        return next;
    }, [configured, handleAcknowledge, handleConnect, handleExport, handleImport, handleOpenBackups, handlePull, handlePush, incident, form.provider]);

    return {
        commands,
        selectedIndex,
        setSelectedIndex,
        surfaceById,
    };
}

import { useCallback, useEffect, useMemo, useRef, useState } from 'preact/hooks';
import { providerFieldSurfaceId } from './form.js';
import { connectActionLabel } from './summary.js';

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

    const surfaces = useMemo(() => {
        const next = [];
        const add = (id, kind, extra = {}) => {
            next.push({ id, kind, index: next.length, ...extra });
        };

        if (configured) {
            add('pull', 'action', { label: 'Pull Now', run: syncBusy ? null : handlePull });
            add('push', 'action', { label: 'Push Now', run: syncBusy ? null : handlePush });
        }
        if (incident) {
            add('acknowledge', 'action', { label: 'Acknowledge', variant: 'btn-primary', run: syncBusy ? null : handleAcknowledge });
        }
        if (configured) {
            add('disconnect', 'action', { label: 'Disconnect', variant: 'btn-ghost', run: syncBusy ? null : handleDisconnect });
        }
        if (!configured) {
            add('connect', 'action', { label: connectActionLabel(configured, form.provider), variant: 'btn-primary', run: syncBusy ? null : handleConnect });
        }
        add('settings', 'toggle', { run: null });
        add('provider', 'field');
        basicProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        advancedProviderFields.forEach(field => add(providerFieldSurfaceId(field.key), 'field'));
        add('export', 'action', { label: 'Export', variant: 'btn-ghost', run: importBusy ? null : handleExport });
        add('import', 'action', { label: 'Import', variant: 'btn-ghost', run: importBusy ? null : handleImport });
        add('open-backups', 'action', { label: 'Open Folder', run: handleOpenBackups });
        backups.forEach(backup => {
            add(`backup:${backup.file_name}`, 'action', { label: backup.file_name, run: () => handlePreviewBackup(backup.file_name) });
        });

        return next;
    }, [
        advancedProviderFields, backups, basicProviderFields, configured,
        form.pull_on_launch, form.push_on_change,
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

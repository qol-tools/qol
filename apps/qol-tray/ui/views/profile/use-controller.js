import { useProfileKeyHandler } from './key-router.js';
import { useBackups } from './use-backups.js';
import { useSurfaceNav } from './use-surface-nav.js';
import { useSyncActions } from './use-sync-actions.js';
import { useSyncForm } from './use-sync-form.js';

export function useProfileController({
    syncStatus,
    syncProviders,
    onSyncStatusChange,
    refreshSyncStatus,
}) {
    const syncForm = useSyncForm({
        syncStatus,
        syncProviders,
        onSyncStatusChange,
    });
    const backups = useBackups({
        incident: syncForm.incident,
        syncStatus,
    });
    const syncActions = useSyncActions({
        activeProvider: syncForm.activeProvider,
        applySyncStatus: syncForm.applySyncStatus,
        form: syncForm.form,
        refreshSyncStatus,
    });
    const surfaceNav = useSurfaceNav({
        advancedProviderFields: syncForm.advancedProviderFields,
        backups: backups.backups,
        basicProviderFields: syncForm.basicProviderFields,
        configured: syncForm.configured,
        form: syncForm.form,
        handleAcknowledge: syncActions.handleAcknowledge,
        handleConnect: syncActions.handleConnect,
        handleDisconnect: syncActions.handleDisconnect,
        handleExport: syncActions.handleExport,
        handleImport: syncActions.handleImport,
        handleOpenBackups: backups.handleOpenBackups,
        handlePreviewBackup: backups.handlePreviewBackup,
        handlePull: syncActions.handlePull,
        handlePush: syncActions.handlePush,
        importBusy: syncActions.importBusy,
        incident: syncForm.incident,
        syncBusy: syncActions.syncBusy,
        updateForm: syncForm.updateForm,
    });
    const keyHandler = useProfileKeyHandler(surfaceNav);

    return {
        advancedProviderFields: syncForm.advancedProviderFields,
        backupPreview: backups.backupPreview,
        backups: backups.backups,
        basicProviderFields: syncForm.basicProviderFields,
        branchOptions: syncForm.branchOptions,
        commands: surfaceNav.commands,
        configured: syncForm.configured,
        form: syncForm.form,
        handleKey: keyHandler.handleKey,
        handlePreviewBackup: backups.handlePreviewBackup,
        incident: syncForm.incident,
        isBlocking: keyHandler.isBlocking,
        lastImport: syncActions.lastImport,
        providerLabels: syncForm.providerLabels,
        providerOptions: syncForm.providerOptions,
        selectedIndex: surfaceNav.selectedIndex,
        setBackupPreview: backups.setBackupPreview,
        setSelectedIndex: surfaceNav.setSelectedIndex,
        surfaceById: surfaceNav.surfaceById,
        syncStatus,
        updateForm: syncForm.updateForm,
    };
}

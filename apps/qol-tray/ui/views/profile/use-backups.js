import { useCallback, useEffect, useState } from 'preact/hooks';
import { toast } from '../../lib/toast.js';
import {
    fetchProfileBackupPreview,
    fetchProfileBackups,
    openProfileBackupsDir,
} from './actions.js';

export function useBackups({ incident, syncStatus }) {
    const [backups, setBackups] = useState([]);
    const [backupPreview, setBackupPreview] = useState(null);

    const refreshBackups = useCallback(async () => {
        try {
            const nextBackups = await fetchProfileBackups();
            setBackups(Array.isArray(nextBackups) ? nextBackups : []);
        } catch (error) {
            toast('error', `Failed to load backups: ${error.message}`);
        }
    }, []);

    useEffect(() => {
        void refreshBackups();
    }, [incident?.backup_file, refreshBackups, syncStatus?.backup_count, syncStatus?.latest_backup_file]);

    const handleOpenBackups = useCallback(async () => {
        try {
            await openProfileBackupsDir();
        } catch (error) {
            toast('error', `Failed to open backups folder: ${error.message}`);
        }
    }, []);

    const handlePreviewBackup = useCallback(async (fileName) => {
        try {
            const preview = await fetchProfileBackupPreview(fileName);
            setBackupPreview(preview);
        } catch (error) {
            toast('error', `Failed to preview backup: ${error.message}`);
        }
    }, []);

    return {
        backupPreview,
        backups,
        handleOpenBackups,
        handlePreviewBackup,
        setBackupPreview,
    };
}

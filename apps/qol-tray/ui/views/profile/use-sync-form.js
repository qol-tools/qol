import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import {
    FIELD_SECTION_ADVANCED,
    FIELD_SECTION_BASIC,
    buildProviderLabels,
    buildProviderOptions,
    createSyncForm,
    providerFields,
} from './form.js';

export function useSyncForm({ syncStatus, syncProviders, onSyncStatusChange }) {
    const [formDirty, setFormDirty] = useState(false);
    const [form, setForm] = useState(() => createSyncForm(syncStatus));
    const configured = Boolean(syncStatus?.configured);
    const incident = syncStatus?.incident || null;
    const providerOptions = useMemo(() => buildProviderOptions(syncProviders, form.provider), [form.provider, syncProviders]);
    const providerLabels = useMemo(() => buildProviderLabels(syncProviders, form.provider), [form.provider, syncProviders]);
    const activeProvider = useMemo(
        () => providerOptions.find(provider => provider.kind === form.provider) || providerOptions[0] || null,
        [form.provider, providerOptions]
    );
    const basicProviderFields = useMemo(
        () => providerFields(activeProvider, FIELD_SECTION_BASIC),
        [activeProvider]
    );
    const advancedProviderFields = useMemo(
        () => providerFields(activeProvider, FIELD_SECTION_ADVANCED),
        [activeProvider]
    );

    useEffect(() => {
        if (formDirty) {
            return;
        }
        setForm(createSyncForm(syncStatus));
    }, [formDirty, syncStatus, syncStatusSeed(syncStatus)]);

    useEffect(() => {
        if (providerOptions.some(provider => provider.kind === form.provider)) {
            return;
        }
        const nextProvider = providerOptions[0]?.kind;
        if (!nextProvider) {
            return;
        }
        setForm(current => ({ ...current, provider: nextProvider }));
    }, [form.provider, providerOptions]);

    const applySyncStatus = useCallback((status) => {
        onSyncStatusChange?.(status);
        setForm(createSyncForm(status));
        setFormDirty(false);
    }, [onSyncStatusChange]);

    const updateForm = useCallback((key, value) => {
        setForm(current => ({ ...current, [key]: value }));
        setFormDirty(true);
    }, []);

    return {
        activeProvider,
        advancedProviderFields,
        applySyncStatus,
        basicProviderFields,
        configured,
        form,
        incident,
        providerLabels,
        providerOptions,
        updateForm,
    };
}

function syncStatusSeed(syncStatus) {
    return [
        syncStatus?.configured ? '1' : '0',
        syncStatus?.provider || '',
        syncStatus?.gist_id || '',
        syncStatus?.folder_path || '',
        syncStatus?.path || '',
        syncStatus?.pull_on_launch ? '1' : '0',
        syncStatus?.push_on_change ? '1' : '0',
    ].join('|');
}

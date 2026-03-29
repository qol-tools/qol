import { useCallback, useEffect, useMemo, useState } from 'preact/hooks';
import { fetchProfileBranches } from './actions.js';
import {
    DEFAULT_BRANCH,
    FIELD_SECTION_ADVANCED,
    FIELD_SECTION_BASIC,
    adoptDefaultBranch,
    buildProviderLabels,
    buildProviderOptions,
    createSyncForm,
    defaultBranchOptions,
    providerFields,
    providerUsesBranchOptions,
} from './form.js';

export function useSyncForm({ syncStatus, syncProviders, onSyncStatusChange }) {
    const [branchOptions, setBranchOptions] = useState(() => defaultBranchOptions(syncStatus?.branch));
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

    useEffect(() => {
        if (!providerUsesBranchOptions(activeProvider)) {
            setBranchOptions(defaultBranchOptions(DEFAULT_BRANCH, syncStatus?.branch));
            return;
        }
        const repoUrl = form.repo_url.trim();
        const token = form.token.trim();
        const fallback = defaultBranchOptions(form.branch, syncStatus?.branch);
        if (!repoUrl) {
            setBranchOptions(fallback);
            return;
        }
        if (!token && !syncStatus?.has_github_token) {
            setBranchOptions(fallback);
            return;
        }

        let cancelled = false;
        const timer = window.setTimeout(async () => {
            try {
                const result = await fetchProfileBranches({ repo_url: repoUrl, token });
                if (cancelled) {
                    return;
                }
                setBranchOptions(defaultBranchOptions(result.default_branch, ...result.branches, form.branch));
                setForm(current => adoptDefaultBranch(current, result.default_branch, syncStatus));
            } catch {
                if (cancelled) {
                    return;
                }
                setBranchOptions(fallback);
            }
        }, 250);

        return () => {
            cancelled = true;
            window.clearTimeout(timer);
        };
    }, [
        activeProvider,
        form.branch,
        form.repo_url,
        form.token,
        syncStatus?.branch,
        syncStatus?.configured,
        syncStatus?.has_github_token,
    ]);

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
        branchOptions,
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
        syncStatus?.repo_url || '',
        syncStatus?.folder_path || '',
        syncStatus?.branch || '',
        syncStatus?.path || '',
        syncStatus?.commit_message || '',
        syncStatus?.pull_on_launch ? '1' : '0',
        syncStatus?.push_on_change ? '1' : '0',
    ].join('|');
}

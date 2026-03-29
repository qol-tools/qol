const DEFAULT_BRANCH = 'main';
const DEFAULT_PROVIDER = 'github';
const FIELD_SECTION_BASIC = 'basic';
const FIELD_SECTION_ADVANCED = 'advanced';
const FIELD_KIND_SELECT = 'select';
const FIELD_KIND_PASSWORD = 'password';
const FIELD_OPTIONS_GITHUB_BRANCHES = 'github_branches';

export function createSyncForm(syncStatus) {
    return {
        provider: syncStatus?.provider || DEFAULT_PROVIDER,
        token: '',
        repo_url: syncStatus?.repo_url || '',
        folder_path: syncStatus?.folder_path || '',
        branch: syncStatus?.branch || DEFAULT_BRANCH,
        path: syncStatus?.path || '',
        commit_message: syncStatus?.commit_message || '',
        pull_on_launch: syncStatus?.pull_on_launch ?? true,
        push_on_change: syncStatus?.push_on_change ?? true,
    };
}

export function buildProviderOptions(syncProviders, selectedProvider) {
    if (Array.isArray(syncProviders) && syncProviders.length > 0) {
        return syncProviders;
    }
    if (selectedProvider) {
        return [{ kind: selectedProvider, label: providerFallbackLabel(selectedProvider), fields: [] }];
    }
    return [{ kind: DEFAULT_PROVIDER, label: providerFallbackLabel(DEFAULT_PROVIDER), fields: [] }];
}

export function buildProviderLabels(syncProviders, selectedProvider) {
    return Object.fromEntries(
        buildProviderOptions(syncProviders, selectedProvider).map(provider => [provider.kind, provider.label || providerFallbackLabel(provider.kind)])
    );
}

export function providerFields(provider, section) {
    if (!provider?.fields) {
        return [];
    }
    return provider.fields.filter(field => field.section === section);
}

export function providerUsesBranchOptions(provider) {
    if (!provider?.fields) {
        return false;
    }
    return provider.fields.some(field => field.options_source === FIELD_OPTIONS_GITHUB_BRANCHES);
}

export function providerFallbackLabel(kind) {
    if (!kind) {
        return 'Sync Target';
    }
    if (kind === 'github') {
        return 'GitHub';
    }
    if (kind === 'folder') {
        return 'Folder';
    }
    return kind
        .split('_')
        .map(part => part ? `${part[0].toUpperCase()}${part.slice(1)}` : '')
        .join(' ');
}

export function providerFieldSurfaceId(key) {
    return `field:${key}`;
}

export function providerFieldInputId(key) {
    return `profile-${String(key).replaceAll('_', '-')}`;
}

export function fieldValue(form, key) {
    if (typeof form?.[key] === 'string') {
        return form[key];
    }
    return '';
}

export function fieldHint(field) {
    return field.hint || '';
}

export function fieldPlaceholder(field, syncStatus) {
    if (field.key === 'token' && syncStatus?.has_github_token) {
        return 'Stored PAT on file';
    }
    return field.placeholder || '';
}

export function fieldOptions(field, form, branchOptions) {
    if (field.options_source === FIELD_OPTIONS_GITHUB_BRANCHES) {
        return branchOptions;
    }
    const value = fieldValue(form, field.key);
    if (value) {
        return [value];
    }
    return [];
}

export function fieldLabels(field, options, form) {
    if (field.options_source === FIELD_OPTIONS_GITHUB_BRANCHES) {
        return branchLabels(options, fieldValue(form, field.key));
    }
    return Object.fromEntries(options.map(option => [option, option]));
}

export function buildConnectPayload(form, provider) {
    const payload = {
        provider: form?.provider || provider?.kind || DEFAULT_PROVIDER,
        pull_on_launch: form?.pull_on_launch ?? true,
        push_on_change: form?.push_on_change ?? true,
    };
    if (!provider?.fields) {
        return payload;
    }
    provider.fields.forEach((field) => {
        payload[field.key] = form?.[field.key] || '';
    });
    return payload;
}

export function defaultBranchOptions(...values) {
    const next = [];
    for (const value of values) {
        const trimmed = typeof value === 'string' ? value.trim() : '';
        if (!trimmed) {
            continue;
        }
        if (next.includes(trimmed)) {
            continue;
        }
        next.push(trimmed);
    }
    if (next.length > 0) {
        return next;
    }
    return [DEFAULT_BRANCH];
}

export function branchLabels(options, selectedBranch) {
    const defaultBranch = options[0] || DEFAULT_BRANCH;
    return Object.fromEntries(options.map((branch) => {
        if (branch === defaultBranch) {
            return [branch, `${branch} · default`];
        }
        if (branch === selectedBranch) {
            return [branch, `${branch}`];
        }
        return [branch, branch];
    }));
}

export function adoptDefaultBranch(current, defaultBranch, syncStatus) {
    if (syncStatus?.configured) {
        return current;
    }
    const nextBranch = defaultBranch?.trim();
    if (!nextBranch) {
        return current;
    }
    const currentBranch = current.branch?.trim();
    if (currentBranch && currentBranch !== DEFAULT_BRANCH) {
        return current;
    }
    if (currentBranch === nextBranch) {
        return current;
    }
    return {
        ...current,
        branch: nextBranch,
    };
}

export {
    DEFAULT_BRANCH,
    FIELD_KIND_PASSWORD,
    FIELD_KIND_SELECT,
    FIELD_OPTIONS_GITHUB_BRANCHES,
    FIELD_SECTION_ADVANCED,
    FIELD_SECTION_BASIC,
};

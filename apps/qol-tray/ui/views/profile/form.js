const DEFAULT_PROVIDER = 'github';
const FIELD_SECTION_BASIC = 'basic';
const FIELD_SECTION_ADVANCED = 'advanced';
const FIELD_KIND_SELECT = 'select';
const FIELD_KIND_PASSWORD = 'password';

export function createSyncForm(syncStatus) {
    return {
        provider: syncStatus?.provider || DEFAULT_PROVIDER,
        gist_id: syncStatus?.gist_id || '',
        folder_path: syncStatus?.folder_path || '',
        path: syncStatus?.path || '',
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
    return field.placeholder || '';
}

export function fieldOptions(field, form) {
    const value = fieldValue(form, field.key);
    if (value) {
        return [value];
    }
    return [];
}

export function fieldLabels(field, options) {
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

export {
    FIELD_KIND_PASSWORD,
    FIELD_KIND_SELECT,
    FIELD_SECTION_ADVANCED,
    FIELD_SECTION_BASIC,
};

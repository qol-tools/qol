import { createContext } from 'preact';
import { useCallback, useEffect, useMemo, useContext, useState } from 'preact/hooks';
import { html } from '../../lib/html.js';
import { usePluginConfig } from './usePluginConfig.js';
import { usePersistedIndex } from '../../hooks/usePersistedIndex.js';
import {
    buildBranchOwnerMap,
    collectVariantGroups,
    isFieldVisible,
} from '../../auto-config/display-rules.js';

const PluginConfigContext = createContext(null);

export function usePluginConfigContext() {
    return useContext(PluginConfigContext);
}

export function PluginConfigProvider({ pluginId, children }) {
    if (!pluginId) return html`<${PluginConfigContext.Provider} value=${null}>${children}<//>`;
    return html`<${ActivePluginConfigProvider} pluginId=${pluginId}>${children}<//>`;
}

function ActivePluginConfigProvider({ pluginId, children }) {
    const config = usePluginConfig(pluginId);
    const [activeSectionIndex, setActiveSectionIndex, , markRestored] = usePersistedIndex(
        `plugin-config-section-${pluginId}`, 0,
    );
    const [selectedFieldIds, setSelectedFieldIds] = useState({});

    useEffect(() => {
        if (config.loading) return;
        markRestored();
    }, [config.loading, markRestored]);

    const navigate = useCallback((delta) => {
        if (config.sections.length === 0) return;
        setActiveSectionIndex(i => (i + delta + config.sections.length) % config.sections.length);
    }, [config.sections.length]);

    const safeIndex = config.sections.length > 0
        ? Math.min(activeSectionIndex, config.sections.length - 1) : 0;
    const activeSection = config.sections[safeIndex] || null;
    const visibleFields = useMemo(
        () => collectVisibleFields(activeSection, config.getFieldValue, config.getFieldValueById),
        [activeSection, config.renderTick]
    );
    const fieldIndexById = useMemo(
        () => Object.fromEntries(visibleFields.map((field, index) => [field.id, index])),
        [visibleFields]
    );
    const selectedFieldId = activeSection?.id ? selectedFieldIds[activeSection.id] ?? null : null;
    const selectedField = visibleFields.find(field => field.id === selectedFieldId) || visibleFields[0] || null;

    useEffect(() => {
        if (!activeSection?.id) return;
        if (visibleFields.length === 0) return;
        if (selectedFieldId && visibleFields.some(field => field.id === selectedFieldId)) return;
        setSelectedFieldIds(current => {
            if (current[activeSection.id] === visibleFields[0].id) return current;
            return { ...current, [activeSection.id]: visibleFields[0].id };
        });
    }, [activeSection?.id, selectedFieldId, visibleFields]);
    const setSelectedFieldId = useCallback((fieldId) => {
        if (!activeSection?.id || !fieldId) return;
        setSelectedFieldIds(current => {
            if (current[activeSection.id] === fieldId) return current;
            return { ...current, [activeSection.id]: fieldId };
        });
    }, [activeSection?.id]);

    const value = useMemo(() => ({
        ...config,
        activeSectionIndex: safeIndex,
        setActiveSectionIndex,
        activeSection,
        navigate,
        visibleFields,
        fieldIndexById,
        selectedFieldId: selectedField?.id || null,
        selectedField,
        setSelectedFieldId,
    }), [
        config,
        safeIndex,
        activeSection,
        navigate,
        visibleFields,
        fieldIndexById,
        selectedField,
        setSelectedFieldId,
    ]);

    return html`<${PluginConfigContext.Provider} value=${value}>${children}<//>`;
}

function collectVisibleFields(section, getFieldValue, getFieldValueById) {
    if (!section?.fields?.length) return [];

    const groups = collectVariantGroups(section.fields);
    const selectorIds = new Set(groups.map(group => group.selector.id));
    const branchOwners = buildBranchOwnerMap(groups);
    const rendered = new Set();
    const visible = [];

    for (const field of section.fields) {
        if (selectorIds.has(field.id)) continue;

        const owner = branchOwners.get(field.id);
        if (owner) {
            if (rendered.has(owner.selector.id)) continue;
            rendered.add(owner.selector.id);
            visible.push(owner.selector);

            const activeOption = getFieldValue(owner.selector);
            for (const branchField of owner.fields) {
                if (branchField.show_when?.equals !== activeOption) continue;
                visible.push(branchField);
            }
            continue;
        }

        if (!isFieldVisible(field, getFieldValueById)) continue;
        visible.push(field);
    }

    return visible;
}

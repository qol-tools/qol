import { createContext } from 'preact';
import { useCallback, useEffect, useMemo, useContext, useState } from 'preact/hooks';
import { html } from '../../lib/html.js';
import { usePluginConfig } from './usePluginConfig.js';
import {
    buildBranchOwnerMap,
    collectVariantGroups,
    isFieldVisible,
} from '../../auto-config/display-rules.js';

const PluginConfigContext = createContext(null);

export function usePluginConfigContext() {
    return useContext(PluginConfigContext);
}

export function PluginConfigProvider({ pluginId, mode, activeSectionId, children }) {
    if (!pluginId) return html`<${PluginConfigContext.Provider} value=${null}>${children}<//>`;
    if (mode === 'ui') return html`<${PluginConfigContext.Provider} value=${{ pluginId, mode }}>${children}<//>`;
    return html`<${ActivePluginConfigProvider} pluginId=${pluginId} mode=${mode} activeSectionId=${activeSectionId}>${children}<//>`;
}

function ActivePluginConfigProvider({ pluginId, mode, activeSectionId, children }) {
    const config = usePluginConfig(pluginId);
    const [selectedFieldIds, setSelectedFieldIds] = useState({});
    const [statusTones, setStatusTones] = useState({});

    const reportStatusTone = useCallback((fieldId, tone) => {
        setStatusTones(current => {
            if (current[fieldId] === tone) return current;
            return { ...current, [fieldId]: tone };
        });
    }, []);

    const isRuntimeDisabled = useMemo(() => {
        return Object.values(statusTones).some(t => t === 'danger');
    }, [statusTones]);

    const activeSection = useMemo(() => {
        const sections = config.sections;
        if (!sections?.length) return null;
        if (activeSectionId) {
            const found = sections.find(s => s.id === activeSectionId);
            if (found) return found;
        }
        return sections[0];
    }, [config.sections, activeSectionId]);

    const visibleFields = useMemo(
        () => collectVisibleFields(activeSection, config.getFieldValue, config.getFieldValueById),
        [activeSection, config.renderTick]
    );
    const fieldIndexById = useMemo(
        () => Object.fromEntries(visibleFields.map((field, index) => [field.id, index])),
        [visibleFields]
    );

    const storedFieldId = activeSection?.id ? selectedFieldIds[activeSection.id] : null;
    const selectedField = visibleFields.find(field => field.id === storedFieldId)
        || visibleFields[0]
        || null;

    useEffect(() => {
        if (!activeSection?.id || visibleFields.length === 0) return;
        if (storedFieldId && visibleFields.some(f => f.id === storedFieldId)) return;
        setSelectedFieldIds(current => ({ ...current, [activeSection.id]: visibleFields[0].id }));
    }, [activeSection?.id, storedFieldId, visibleFields]);

    const setSelectedFieldId = useCallback((fieldId) => {
        if (!activeSection?.id || !fieldId) return;
        setSelectedFieldIds(current => {
            if (current[activeSection.id] === fieldId) return current;
            return { ...current, [activeSection.id]: fieldId };
        });
    }, [activeSection?.id]);

    const value = useMemo(() => ({
        ...config,
        pluginId,
        mode,
        activeSection,
        visibleFields,
        fieldIndexById,
        selectedFieldId: selectedField?.id || null,
        selectedField,
        setSelectedFieldId,
        reportStatusTone,
        isRuntimeDisabled,
    }), [config, pluginId, mode, activeSection, visibleFields, fieldIndexById, selectedField, setSelectedFieldId, reportStatusTone, isRuntimeDisabled]);

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

import { html } from '../../lib/html.js';
import { useCallback, useRef } from 'preact/hooks';
import { usePluginConfigContext } from './context.js';
import { prettyLabel } from '../../auto-config/heuristics.js';
import {
    buildBranchOwnerMap,
    collectVariantGroups,
    isFieldVisible,
    optionLabel,
    selectorDensityClass,
    selectorGridTemplate,
} from '../../auto-config/display-rules.js';
import { renderField, fieldSelectionClasses } from './field-map.js';
import { dissolveIn, DISSOLVE_PRESETS } from '../../lib/dissolve.js';

export function PluginConfigView({ onClose }) {
    const ctx = usePluginConfigContext();

    if (ctx?.mode === 'ui') {
        return html`
            <div class="plugin-ui-container">
                <div class="plugin-ui-toolbar">
                    <span class="plugin-ui-toolbar-title">${ctx.pluginId}</span>
                    <button class="plugin-ui-toolbar-close" onClick=${onClose}>\u00d7</button>
                </div>
                <iframe
                    src=${`/plugins/${ctx.pluginId}/`}
                    class="plugin-custom-ui"
                />
            </div>
        `;
    }

    const section = ctx?.activeSection;

    if (!ctx || ctx.loading) return html`<div class="plugin-config-loading">Loading configuration...</div>`;
    if (ctx.sections.length === 0) return html`<div class="plugin-config-loading">No settings available.</div>`;

    return html`
        <div class="plugin-config-detail" tabIndex="-1" data-surface-container="">
            ${section && html`
                <div class="config-detail-content">
                    <header class="config-detail-header">
                        <h2>${section.label || prettyLabel(section.id)}</h2>
                        ${section.description && html`<p class="section-copy">${section.description}</p>`}
                    </header>
                    <${ConfigSection} fields=${section.fields} />
                </div>
            `}
        </div>
    `;
}

function ConfigSection({ fields }) {
    const ctx = usePluginConfigContext();
    const groups = collectVariantGroups(fields);
    const selectorIds = new Set(groups.map(group => group.selector.id));
    const branchOwners = buildBranchOwnerMap(groups);
    const rendered = new Set();

    return html`
        <div class="config-section">
            ${fields.map(field => {
                if (selectorIds.has(field.id)) return null;
                const owner = branchOwners.get(field.id);
                if (owner) {
                    if (rendered.has(owner.selector.id)) return null;
                    rendered.add(owner.selector.id);
                    return html`<${VariantPanel} key=${owner.selector.id} group=${owner} />`;
                }
                if (!isFieldVisible(field, fieldId => ctx.getFieldValueById(fieldId))) return null;
                return renderField(field);
            })}
        </div>
    `;
}

function VariantPanel({ group }) {
    const ctx = usePluginConfigContext();
    const contentRef = useRef(null);
    const activeOption = ctx.getFieldValue(group.selector);
    const selected = ctx.selectedFieldId === group.selector.id;
    const index = ctx.fieldIndexById[group.selector.id];
    const densityClass = selectorDensityClass(group.selector);
    const widthStyle = `grid-template-columns: ${selectorGridTemplate(group.selector)}`;

    const onSelect = useCallback((option) => {
        if (option === ctx.getFieldValue(group.selector)) return;
        ctx.setFieldValue(group.selector, option);
        ctx.bumpRender();
        ctx.save();
        if (contentRef.current) {
            dissolveIn(contentRef.current, DISSOLVE_PRESETS.variantSwitch);
        }
    }, [group, ctx]);
    const onFocusSelector = useCallback(() => {
        ctx.setSelectedFieldId(group.selector.id);
    }, [ctx, group.selector.id]);

    return html`
        <div class="variant-panel">
            <div class="variant-selector ${densityClass} ${fieldSelectionClasses(selected)}"
                data-plugin-config-field-id=${group.selector.id}
                data-plugin-config-index=${index}
                data-selected-surface=""
                data-selected=${selected ? 'true' : 'false'}
                onMouseDown=${onFocusSelector}
                onFocus=${onFocusSelector}>
                <div class="variant-selector-label">${group.selector.label}</div>
                <div class="variant-selector-card">
                    <div class="variant-selector-options segmented-control" style=${widthStyle}>
                        ${group.selector.options.map(option => html`
                            <button key=${option} type="button"
                                class="variant-option segmented-control__option ${option === activeOption ? 'active is-active' : ''}"
                                tabIndex="-1"
                                onClick=${() => onSelect(option)}>
                                ${optionLabel(group.selector, option)}
                            </button>
                        `)}
                    </div>
                </div>
            </div>
            <div class="variant-content" ref=${contentRef}>
                ${group.selector.options.map(option => html`
                    <div key=${option} class="variant-content-branch ${option === activeOption ? 'active' : ''}">
                        ${group.fields
                            .filter(field => field.show_when?.equals === option)
                            .map(renderField)}
                    </div>
                `)}
            </div>
        </div>
    `;
}

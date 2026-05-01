import { html } from '../../lib/html.js';
import { useCallback, useEffect, useRef } from 'preact/hooks';
import { usePluginConfigContext } from './context.js';
import {
    buildBranchOwnerMap,
    collectVariantGroups,
    isFieldVisible,
    optionLabel,
    selectorDensityClass,
    selectorGridTemplate,
} from '../../auto-config/display-rules.js';
import { renderField, fieldSurfaceAttrs } from './field-map.js';
import { dissolveIn, DISSOLVE_PRESETS } from '../../lib/dissolve.js';
import { SurfaceContainer } from '../../lib/components/SurfaceContainer.js';

function useEscapeFallback(onClose, active) {
    useEffect(() => {
        if (!active) return undefined;
        const onKey = (event) => {
            if (event.key !== 'Escape') return;
            event.preventDefault();
            event.stopPropagation();
            onClose();
        };
        document.addEventListener('keydown', onKey, true);
        return () => document.removeEventListener('keydown', onKey, true);
    }, [onClose, active]);
}

export function PluginConfigView({ onClose }) {
    const ctx = usePluginConfigContext();
    const isPlaceholder = !ctx || ctx.loading || (ctx && ctx.sections && ctx.sections.length === 0);
    useEscapeFallback(onClose, isPlaceholder);

    const section = ctx?.activeSection;

    if (!ctx || ctx.loading) {
        return html`
            <div class="plugin-config-loading" onClick=${onClose} title="Press Escape or click to return">
                Loading configuration...
            </div>
        `;
    }
    if (ctx.sections.length === 0) {
        return html`
            <div class="plugin-config-loading" onClick=${onClose} title="Press Escape or click to return">
                No settings available.
            </div>
        `;
    }

    return html`
        <${SurfaceContainer} className="plugin-config-detail" tabIndex="-1">
            ${section && html`
                <div class="config-detail-content">
                    ${section.id !== '_root' && section.description && html`
                        <p class="section-copy">${section.description}</p>
                    `}
                    <${ConfigSection} fields=${section.fields} />
                </div>
            `}
        <//>
    `;
}

export function PluginConfigSectionView({ pluginId, sectionId, onClose }) {
    const ctx = usePluginConfigContext();
    if (!ctx || ctx.pluginId !== pluginId) return null;
    if (ctx.loading || !ctx.sections) return null;
    const section = ctx.sections.find(s => s.id === sectionId);
    if (!section) return null;
    return html`
        <${SurfaceContainer} className="plugin-config-detail" tabIndex="-1">
            <div class="config-detail-content">
                ${section.id !== '_root' && section.description && html`
                    <p class="section-copy">${section.description}</p>
                `}
                <${ConfigSection} fields=${section.fields} />
            </div>
        <//>
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
            <div ...${fieldSurfaceAttrs(group.selector, ctx, `variant-selector ${densityClass}`)}
                onMouseDown=${onFocusSelector}
                onFocus=${onFocusSelector}>
                <div class="variant-selector-label">${group.selector.label}</div>
                <div class="variant-selector-card">
                    <div class="variant-selector-options segmented-control" style=${widthStyle}>
                        ${group.selector.options.map(option => html`
                            <button key=${option} type="button"
                                class="variant-option segmented-control__option ${option === activeOption ? 'active is-active' : ''}"
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

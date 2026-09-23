import { getFieldValue, setFieldValue } from './normalized-config.js';
import {
    buildBranchOwnerMap,
    optionLabel,
    selectorDensityClass,
    selectorGridTemplate,
} from './display-rules.js';

let _dissolveIn = null;

export function setDissolveIn(fn) { _dissolveIn = fn; }

export function appendVariantSelectors(context) {
    if (context.groups.length === 0) {
        return;
    }
    context._variantSelectors = new Map();
    for (const item of context.groups) {
        context._variantSelectors.set(item.selector.id, renderVariantSelector(item.selector, context));
    }
}

export function renderVariantFields(container, context) {
    const selectorIds = new Set(context.groups.map(group => group.selector.id));
    const branchOwners = buildBranchOwnerMap(context.groups);
    const renderedBranches = new Set();

    for (const field of context.fields) {
        if (selectorIds.has(field.id)) {
            continue;
        }

        const owner = branchOwners.get(field.id);
        if (owner) {
            if (renderedBranches.has(owner.selector.id)) {
                continue;
            }
            renderedBranches.add(owner.selector.id);
            container.appendChild(renderVariantPanel(owner, context));
            continue;
        }

        if (!context.isFieldVisible(field)) {
            continue;
        }

        container.appendChild(context.renderField(field));
    }
}

function renderVariantPanel(group, context) {
    const panel = document.createElement('div');
    panel.className = 'variant-panel';

    const selector = context._variantSelectors?.get(group.selector.id);
    if (selector) {
        panel.appendChild(selector);
    }

    const content = document.createElement('div');
    content.className = 'variant-content';
    content.dataset.variantField = group.selector.id;

    group.selector.options.forEach(option => {
        const branch = document.createElement('div');
        branch.className = 'variant-content-branch';
        branch.dataset.variantOption = option;
        if (getFieldValue(context.state, group.selector) === option) {
            branch.classList.add('active');
        }
        const fields = group.fields.filter(field => field.show_when?.equals === option);
        for (const field of fields) {
            branch.appendChild(context.renderField(field));
        }
        content.appendChild(branch);
    });

    panel.appendChild(content);
    return panel;
}

function renderVariantSelector(field, context) {
    const group = document.createElement('div');
    group.className = 'variant-selector';

    const densityClass = selectorDensityClass(field);
    if (densityClass) {
        group.classList.add(densityClass);
    }

    const label = document.createElement('div');
    label.className = 'variant-selector-label';
    label.textContent = field.label;
    group.appendChild(label);

    const card = document.createElement('div');
    card.className = 'variant-selector-card';

    const options = document.createElement('div');
    options.className = 'variant-selector-options segmented-control';
    options.style.setProperty('--variant-option-count', `${field.options.length}`);
    options.style.gridTemplateColumns = selectorGridTemplate(field);

    for (const option of field.options) {
        options.appendChild(renderVariantOption(field, option, context));
    }

    card.appendChild(options);
    group.appendChild(card);
    return group;
}

function renderVariantOption(field, option, context) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'variant-option segmented-control__option';
    button.textContent = optionLabel(field, option);
    button.dataset.variantField = field.id;
    button.dataset.variantOption = option;
    if (option === getFieldValue(context.state, field)) {
        button.classList.add('active', 'is-active');
    }
    button.addEventListener('click', () => {
        if (getFieldValue(context.state, field) === option) {
            return;
        }
        setFieldValue(context.state, field, option);
        updateVariantSelection(field.id, option);
        context.state.save();
    });
    return button;
}

function updateVariantSelection(fieldId, option) {
    updateVariantButtons(fieldId, option);
    updateVariantBranches(fieldId, option);
}

function updateVariantButtons(fieldId, option) {
    const selector = `.variant-option[data-variant-field="${fieldId}"]`;
    for (const button of document.querySelectorAll(selector)) {
        const isActive = button.dataset.variantOption === option;
        button.classList.toggle('active', isActive);
        button.classList.toggle('is-active', isActive);
    }
}

function updateVariantBranches(fieldId, option) {
    const selector = `.variant-content[data-variant-field="${fieldId}"]`;
    for (const wrapper of document.querySelectorAll(selector)) {
        let changed = false;
        for (const branch of wrapper.querySelectorAll('.variant-content-branch')) {
            const isActive = branch.dataset.variantOption === option;
            if (isActive !== branch.classList.contains('active')) {
                changed = true;
            }
            branch.classList.toggle('active', isActive);
        }
        if (changed) {
            playVariantDissolve(wrapper);
        }
    }
}

function weightedScale() {
    const r = Math.random();
    if (r < 0.5) return 1;
    if (r < 0.8) return 2;
    if (r < 0.95) return 3;
    return 4;
}

function playVariantDissolve(container) {
    if (!_dissolveIn) return;
    const scale = weightedScale();
    _dissolveIn(container, {
        bleed: 20,
        edgeFade: 20,
        filter: `blur(${(scale * 0.4).toFixed(1)}px)`,
        renderScale: scale,
        density: 1.0,
        tileSize: 128,
        dissolveRate: 0.12,
        bubbleFade: 0.065,
        maxBatchRate: 0.1,
        origin: 'center',
    });
}

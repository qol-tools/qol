import { getVal } from './config-paths.js';
import { createFieldLabel } from './field-label.js';

let _iconUrlFor = (value) => `/api/icon/${encodeURIComponent(value)}`;

export function setIconUrl(fn) { _iconUrlFor = fn; }

export function renderStringList(key, path, state) {
    const div = document.createElement('div');
    div.className = 'field-group';
    div.appendChild(createFieldLabel(key));
    const listEl = document.createElement('div');
    listEl.className = 'string-list';
    div.appendChild(listEl);
    const isApps = key === 'excluded_apps' || key.endsWith('_apps') || key.endsWith('_bundles');
    const render = () => renderList(listEl, path, state, isApps, render);
    div.appendChild(buildAddRow(path, state, render));
    render();
    return div;
}

function renderList(listEl, path, state, isApps, render) {
    const values = getVal(state.config, path) || [];
    listEl.replaceChildren();
    if (values.length === 0) {
        const emptyState = document.createElement('div');
        emptyState.className = 'empty-state';
        emptyState.textContent = 'No items.';
        listEl.appendChild(emptyState);
        return;
    }
    values.forEach((item, index) => listEl.appendChild(renderItem(item, index, path, state, isApps, render)));
}

function renderItem(item, index, path, state, isApps, render) {
    const row = document.createElement('div');
    row.className = 'string-item';
    if (isApps) row.appendChild(createAppIcon(item));
    const text = document.createElement('span');
    text.textContent = item;
    row.appendChild(text);
    row.appendChild(createRemoveButton(index, path, state, render));
    return row;
}

function createAppIcon(item) {
    const icon = document.createElement('img');
    icon.className = 'app-icon';
    icon.src = _iconUrlFor(item);
    icon.width = 20;
    icon.height = 20;
    icon.onerror = () => { icon.style.display = 'none'; };
    return icon;
}

function createRemoveButton(index, path, state, render) {
    const button = document.createElement('button');
    button.type = 'button';
    button.className = 'btn-remove';
    button.textContent = '×';
    button.dataset.idx = index;
    button.addEventListener('click', () => {
        getVal(state.config, path).splice(parseInt(button.dataset.idx, 10), 1);
        render();
    });
    return button;
}

function buildAddRow(path, state, render) {
    const addRow = document.createElement('div');
    addRow.className = 'add-row';
    const addInput = document.createElement('input');
    addInput.type = 'text';
    addInput.className = 'text-input';
    addInput.placeholder = 'Add item...';
    const addButton = document.createElement('button');
    addButton.type = 'button';
    addButton.className = 'btn btn-ghost btn-sm btn-add';
    addButton.textContent = '+ Add';
    addButton.addEventListener('click', () => addItem(addInput, path, state, render));
    addInput.addEventListener('keydown', e => { if (e.key === 'Enter') { e.preventDefault(); addItem(addInput, path, state, render); } });
    addRow.append(addInput, addButton);
    return addRow;
}

function addItem(addInput, path, state, render) {
    const nextValue = addInput.value.trim();
    if (!nextValue) return;
    const values = getVal(state.config, path);
    if (values.includes(nextValue)) return;
    values.push(nextValue);
    addInput.value = '';
    render();
}

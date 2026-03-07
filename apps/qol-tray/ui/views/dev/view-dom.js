export function saveSpinnerTimes(root) {
    const times = [];
    for (const button of root.querySelectorAll('.refresh-btn.spinning')) {
        const animation = button.getAnimations?.()[0];
        times.push(animation ? animation.currentTime : null);
    }
    return times;
}

export function restoreSpinnerTimes(root, times) {
    if (!times.length) {
        return;
    }

    const buttons = root.querySelectorAll('.refresh-btn.spinning');
    for (let index = 0; index < buttons.length && index < times.length; index += 1) {
        if (times[index] === null) {
            continue;
        }

        const animation = buttons[index].getAnimations?.()[0];
        if (animation) {
            animation.currentTime = times[index];
        }
    }
}

export function readHoveredActionId(root) {
    const hoveredZone = root.querySelector('.plugin-action-zone:hover');
    return hoveredZone?.dataset.id || null;
}

export function restoreHoveredAction(root, hoveredActionId) {
    if (!hoveredActionId) {
        return;
    }

    const actionZones = root.querySelectorAll('.plugin-action-zone[data-id]');
    for (const zone of actionZones) {
        if (zone.dataset.id !== hoveredActionId) {
            continue;
        }
        if (zone.classList.contains('is-disabled')) {
            continue;
        }

        zone.classList.add('is-hovered');
        return;
    }
}

export function restoreViewBodyScroll(root, scrollTop) {
    const viewBody = root.querySelector('.view-body');
    if (viewBody) {
        viewBody.scrollTop = scrollTop;
    }
}

export function bindLinkInput(root, { onInput, onConfirm, onCancel }) {
    const input = root.querySelector('#link-path');
    if (!input) {
        return;
    }

    input.addEventListener('input', event => {
        onInput(event.target.value);
    });

    input.addEventListener('keydown', event => {
        if (event.key === 'Enter') {
            onConfirm();
        }
        if (event.key === 'Escape') {
            onCancel();
        }
    });
}

export function bindActionInteractionLocks(root, { onEnter, onLeave }) {
    const columns = root.querySelectorAll('.plugin-action-column');
    for (const column of columns) {
        column.addEventListener('pointerenter', onEnter);
        column.addEventListener('pointerleave', onLeave);
    }
}

export function syncPluginMenuDom(root, openPluginMenuId, openCoreMenuId) {
    if (!root) {
        return;
    }

    const pluginRows = root.querySelectorAll('.plugin-row[data-plugin-id]');
    for (const row of pluginRows) {
        const isOpen = row.dataset.pluginId === openPluginMenuId;
        syncMenuRow(row, isOpen);
    }

    const coreRows = root.querySelectorAll('.core-log-row[data-core-section]');
    for (const row of coreRows) {
        const isOpen = row.dataset.coreSection === openCoreMenuId;
        syncMenuRow(row, isOpen);
    }
}

function syncMenuRow(row, isOpen) {
    const menu = row.querySelector('.plugin-context-menu');
    if (menu) {
        menu.classList.toggle('open', isOpen);
    }

    const trigger = row.querySelector('.plugin-menu-trigger');
    if (trigger) {
        trigger.setAttribute('aria-expanded', isOpen ? 'true' : 'false');
    }
}

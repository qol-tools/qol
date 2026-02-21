export function openModal(container, { className, html }) {
    const modal = document.createElement('div');
    modal.className = className;
    modal.innerHTML = html;
    container.appendChild(modal);
    return modal;
}

export function closeModal(container, selector) {
    const modal = container?.querySelector(selector);
    if (!modal) return false;
    modal.remove();
    return true;
}

export function matchModalAction(event, {
    backdropClass,
    cancelSelectors = [],
    confirmSelectors = []
}) {
    if (backdropClass && event.target.classList.contains(backdropClass)) {
        return 'cancel';
    }
    for (const selector of cancelSelectors) {
        if (event.target.closest(selector)) return 'cancel';
    }
    for (const selector of confirmSelectors) {
        if (event.target.closest(selector)) return 'confirm';
    }
    return null;
}

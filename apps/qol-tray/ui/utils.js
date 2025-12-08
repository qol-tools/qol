export function updateSelection(selector, index) {
    document.querySelectorAll(selector).forEach((el, i) => {
        el.classList.toggle('selected', i === index);
    });
    const selected = document.querySelector(`${selector}.selected`);
    if (selected) {
        selected.scrollIntoView({ behavior: 'smooth', block: 'nearest' });
    }
}

export function navigate(state, key, total, delta) {
    if (total === 0) return false;
    const newIndex = Math.max(0, Math.min(total - 1, state[key] + delta));
    if (newIndex !== state[key]) {
        state[key] = newIndex;
        return true;
    }
    return false;
}

export function escapeHtml(value) {
    return String(value).replace(/[&<>"']/g, (char) => (
        {
            '&': '&amp;',
            '<': '&lt;',
            '>': '&gt;',
            '"': '&quot;',
            "'": '&#39;'
        }[char]
    ));
}

export function renderFeedback(targetEl, feedback) {
    if (!targetEl) return;
    if (!feedback) {
        targetEl.innerHTML = '';
        return;
    }
    targetEl.innerHTML = `<div class="view-feedback ${feedback.type}">${escapeHtml(feedback.message)}</div>`;
}

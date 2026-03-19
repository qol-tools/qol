const ENTITY_MAP = {
    '&': '&amp;',
    '<': '&lt;',
    '>': '&gt;',
    '"': '&quot;',
    "'": '&#39;'
};

const ENTITY_PATTERN = /[&<>"']/g;

export function escapeHtml(value) {
    if (value === null || value === undefined) return '';
    return String(value).replace(ENTITY_PATTERN, char => ENTITY_MAP[char] || char);
}

export function safeStatusToken(value) {
    const token = String(value || '').toLowerCase();
    if (token === 'linked') return token;
    if (token === 'installed') return token;
    if (token === 'local') return token;
    return 'local';
}

export function safeStatusToken(value) {
    const token = String(value || '').toLowerCase();
    if (token === 'linked') return token;
    if (token === 'installed') return token;
    if (token === 'local') return token;
    return 'local';
}

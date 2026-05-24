import { apiJson } from '../../api/client.js';

export async function fetchAuthHealth() {
    return apiJson('/api/auth/health');
}

export function hasInsufficientScope(authHealth, provider) {
    if (!authHealth || !Array.isArray(authHealth.issues)) return false;
    return authHealth.issues.some(issue =>
        issue.kind === 'insufficient_scope' && issue.provider === provider
    );
}

export function insufficientScopeIssue(authHealth, provider) {
    if (!authHealth || !Array.isArray(authHealth.issues)) return null;
    return authHealth.issues.find(issue =>
        issue.kind === 'insufficient_scope' && issue.provider === provider
    ) || null;
}

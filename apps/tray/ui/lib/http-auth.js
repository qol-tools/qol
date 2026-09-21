export const AUTH_LOST_EVENT = 'qol:http-auth-lost';
const TOKEN_STORAGE_KEY = 'qol:http-token';
const TOKEN_COOKIE_NAME = 'qol_token';

let authLost = false;

export function isAuthLost() {
    return authLost;
}

export function resetAuthLostState() {
    authLost = false;
}

export function clearTokenEvidence({ storage = null, doc = null, win = null } = {}) {
    try {
        storage?.removeItem(TOKEN_STORAGE_KEY);
    } catch {}
    try {
        if (doc) doc.cookie = `${TOKEN_COOKIE_NAME}=; Max-Age=0; Path=/; SameSite=Strict`;
    } catch {}
    try {
        if (win) win.__QOL_HTTP_TOKEN__ = null;
    } catch {}
}

export function declareAuthLost(env) {
    if (authLost) return false;
    authLost = true;
    clearTokenEvidence(env);
    const win = env?.win ?? null;
    if (win) {
        try {
            win.dispatchEvent(new win.CustomEvent(AUTH_LOST_EVENT));
        } catch {}
    }
    return true;
}

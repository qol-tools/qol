const DEV_PATH_KEY = 'qol:dev-path';
const PROD_PATH_KEY = 'qol:prod-path';

export function pathKeyFor(target) {
    return target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
}

export function loadModePath(target) {
    if (typeof localStorage === 'undefined') return null;
    return localStorage.getItem(pathKeyFor(target));
}

export function saveModePath(target, path) {
    if (typeof localStorage === 'undefined') return;
    localStorage.setItem(pathKeyFor(target), path);
}

export function clearModePath(target) {
    if (typeof localStorage === 'undefined') return;
    localStorage.removeItem(pathKeyFor(target));
}

export async function validateModePath(target, path) {
    const res = await fetch('/api/mode/validate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ target, path }),
    });
    if (!res.ok) return false;
    const valid = await res.json();
    return valid === true;
}

export async function executeModeSwitch(target, path) {
    return fetch('/api/mode/switch', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ target, path }),
    });
}

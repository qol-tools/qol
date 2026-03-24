const DEV_PATH_KEY = 'qol:dev-path';
const PROD_PATH_KEY = 'qol:prod-path';

export function buildModeSwitchCommand({ isDevMode, onNeedPath, onStartSwitch }) {
    const target = isDevMode ? 'prod' : 'dev';
    const key = target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
    const label = isDevMode ? 'Switch to Prod' : 'Switch to Dev';
    return {
        id: 'mode:switch',
        label,
        hidden: true,
        run: () => {
            const saved = localStorage.getItem(key);
            if (saved) {
                onStartSwitch(target, saved);
            } else {
                onNeedPath(target);
            }
        },
    };
}

export function saveModePath(target, path) {
    const key = target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
    localStorage.setItem(key, path);
}

export function clearModePath(target) {
    const key = target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
    localStorage.removeItem(key);
}

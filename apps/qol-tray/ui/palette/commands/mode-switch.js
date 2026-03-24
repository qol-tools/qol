import { GLOBAL_ID, registerCommands } from '../registry.js';

const DEV_PATH_KEY = 'qol:dev-path';
const PROD_PATH_KEY = 'qol:prod-path';

export function registerModeSwitchCommands({ isDevMode, onNeedPath, onStartSwitch }) {
    const commands = [];

    if (isDevMode) {
        commands.push({
            id: 'mode:switch-to-prod',
            label: 'Switch to Prod',
            run: () => {
                const saved = localStorage.getItem(PROD_PATH_KEY);
                if (saved) {
                    onStartSwitch('prod', saved);
                } else {
                    onNeedPath('prod');
                }
            },
        });
    } else {
        commands.push({
            id: 'mode:switch-to-dev',
            label: 'Switch to Dev',
            run: () => {
                const saved = localStorage.getItem(DEV_PATH_KEY);
                if (saved) {
                    onStartSwitch('dev', saved);
                } else {
                    onNeedPath('dev');
                }
            },
        });
    }

    registerCommands(GLOBAL_ID, commands);
}

export function saveModePath(target, path) {
    const key = target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
    localStorage.setItem(key, path);
}

export function clearModePath(target) {
    const key = target === 'dev' ? DEV_PATH_KEY : PROD_PATH_KEY;
    localStorage.removeItem(key);
}

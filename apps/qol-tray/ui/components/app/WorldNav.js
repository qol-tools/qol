import { useCallback, useMemo } from 'preact/hooks';
import { useRegisterCommands } from '../../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../../palette/registry.js';
import { useKeyboard } from '../../hooks/useKeyboard.js';

export function useWorldNav({ camera, registry, viewportRef }) {
    const getViewportSize = useCallback(() => {
        const el = viewportRef?.current;
        return { w: el?.clientWidth || 800, h: el?.clientHeight || 600 };
    }, [viewportRef]);

    const jumpToView = useCallback((id) => {
        const { w, h } = getViewportSize();
        const target = registry.cameraTargetForView(id, w, h);
        if (target) camera.panSmooth(target.x, target.y, 400);
    }, [camera, registry, getViewportSize]);

    const fitAll = useCallback(() => {
        const bounds = registry.worldBounds();
        const { w, h } = getViewportSize();
        camera.panSmooth(
            bounds.x + bounds.width / 2 - w / 2,
            bounds.y + bounds.height / 2 - h / 2,
            400
        );
    }, [camera, registry, getViewportSize]);

    const commands = useMemo(() => {
        const cmds = registry.getAllEntries().map(e => ({
            id: `world:jump:${e.id}`,
            label: `Go to ${formatLabel(e.id)}`,
            run: () => jumpToView(e.id),
        }));
        cmds.push({ id: 'world:fit-all', label: 'Fit all views', run: fitAll });
        return cmds;
    }, [registry, jumpToView, fitAll]);

    useRegisterCommands(GLOBAL_ID, commands);

    useKeyboard(useCallback((e) => {
        if (e.shiftKey && (e.key === '!' || e.key === '1')) {
            e.preventDefault();
            fitAll();
        }
    }, [fitAll]));
}

function formatLabel(id) {
    const labels = {
        plugins: 'Plugins', store: 'Store', hotkeys: 'Hotkeys',
        shortcuts: 'Shortcuts', 'task-runner': 'Task Runner',
        profile: 'Profile', logs: 'Logs', dev: 'Developer',
    };
    return labels[id] || id;
}

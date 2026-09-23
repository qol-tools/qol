import { useCallback, useMemo } from 'preact/hooks';
import { useRegisterCommands } from '../palette/useRegisterCommands.js';
import { GLOBAL_ID } from '../palette/registry.js';
import { useKeyboard } from '../lib/hooks/useKeyboard.js';
import { getViewLabel } from './views.js';

export function useWorldNav({ camera, registry, viewportRef }) {
    const getViewportSize = useCallback(() => {
        const el = viewportRef?.current;
        return { w: el?.clientWidth || 800, h: el?.clientHeight || 600 };
    }, [viewportRef]);

    const jumpToView = useCallback((id) => {
        const { w, h } = getViewportSize();
        const target = registry.cameraTargetForView(id, w, h, camera.zoom);
        if (target) camera.panSmooth(target.x, target.y, 400);
    }, [camera, registry, getViewportSize]);

    const fitAll = useCallback(() => {
        const bounds = registry.worldBounds(0);
        const { w, h } = getViewportSize();
        const z = camera.zoom;
        camera.panSmooth(
            bounds.x + bounds.width / 2 - w / (2 * z),
            bounds.y + bounds.height / 2 - h / (2 * z),
            400
        );
    }, [camera, registry, getViewportSize]);

    const commands = useMemo(() => {
        const cmds = registry.getEntriesForLayer(0).map(e => ({
            id: `world:jump:${e.id}`,
            label: `Go to ${getViewLabel(e.id).text}`,
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


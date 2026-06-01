import { useEffect } from 'preact/hooks';

export function useDiveEditor({ slot, build, deps }) {
    useEffect(() => {
        slot.set(build());
    }, deps);
}

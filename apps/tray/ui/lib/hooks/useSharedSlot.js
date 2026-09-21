import { useEffect, useState } from 'preact/hooks';

export function useSharedSlot(slot) {
    const [, bump] = useState(0);
    useEffect(() => slot.subscribe(() => bump(t => t + 1)), [slot]);
    return slot.get();
}

import { useState, useEffect } from 'preact/hooks';
import * as installing from '../installing.js';

export function useInstalling() {
    const [items, setItems] = useState(() => installing.getAll());
    useEffect(() => installing.subscribe(() => setItems(installing.getAll())), []);
    return { items, has: installing.has, add: installing.add, remove: installing.remove };
}

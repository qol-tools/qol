import { useState, useRef } from 'preact/hooks';

export function useStateRef(initialValue) {
    const [value, setValue] = useState(initialValue);
    const ref = useRef(value);
    ref.current = value;
    return [value, setValue, ref];
}

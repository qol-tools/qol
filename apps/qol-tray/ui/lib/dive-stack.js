export function createDiveStack() {
    const stack = [];

    function push(state) {
        stack.push(state);
    }

    function pop() {
        return stack.pop() || null;
    }

    function peek() {
        return stack[stack.length - 1] || null;
    }

    function clear() {
        stack.length = 0;
    }

    function depth() {
        return stack.length;
    }

    return { push, pop, peek, clear, depth };
}

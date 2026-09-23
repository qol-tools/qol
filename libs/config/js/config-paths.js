export function getVal(config, path) {
    const parts = path.split('.');
    let obj = config;
    for (const part of parts) {
        obj = obj?.[part];
    }
    return obj;
}

export function setVal(config, path, value) {
    const parts = path.split('.');
    let obj = config;
    for (let index = 0; index < parts.length - 1; index += 1) {
        if (obj[parts[index]] === undefined) {
            obj[parts[index]] = {};
        }
        obj = obj[parts[index]];
    }
    obj[parts[parts.length - 1]] = value;
}

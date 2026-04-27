let _diveViaSelector = null;
let _diveFromSurface = null;
let _ascend = null;

export function setDiveViaSelector(fn) {
    _diveViaSelector = fn;
}

export function diveViaSelector(sourceSelector) {
    return _diveViaSelector ? _diveViaSelector(sourceSelector) : false;
}

export function setDiveFromSurface(fn) {
    _diveFromSurface = fn;
}

export function diveFromSurface(surface) {
    return _diveFromSurface ? _diveFromSurface(surface) : false;
}

export function setAscend(fn) {
    _ascend = fn;
}

export function ascend() {
    return _ascend ? _ascend() : false;
}

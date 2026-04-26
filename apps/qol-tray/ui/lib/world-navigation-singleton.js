let _diveViaSelector = null;
let _diveFromSurface = null;

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

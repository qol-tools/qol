let _diveViaSelector = null;

export function setDiveViaSelector(fn) {
    _diveViaSelector = fn;
}

export function diveViaSelector(sourceSelector) {
    return _diveViaSelector ? _diveViaSelector(sourceSelector) : false;
}

"""Canonical JSON matching `tcc_ir` BTreeMap stringify and the TypeScript frontend."""

import math


def canonical_stringify(value: object) -> str:
    return _write(value)


def _write(value: object) -> str:
    if value is None:
        return "null"
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, int) and not isinstance(value, bool):
        return str(value)
    if isinstance(value, float):
        if value != value or value in (float("inf"), float("-inf")):
            return "null"
        if value == 0.0 and math.copysign(1.0, value) < 0:
            return "-0"
        if value.is_integer():
            return str(int(value))
        return repr(value) if value == 0 else format(value, ".15g")
    if isinstance(value, str):
        return _write_string(value)
    if isinstance(value, list):
        return "[" + ",".join(_write(item) for item in value) + "]"
    if isinstance(value, dict):
        entries = sorted(value.items(), key=lambda item: item[0])
        inner = ",".join(f"{_write_string(key)}:{_write(item)}" for key, item in entries)
        return "{" + inner + "}"
    raise TypeError(f"unsupported JSON value: {type(value)!r}")


def _write_string(text: str) -> str:
    out = ['"']
    for ch in text:
        if ch == '"':
            out.append('\\"')
        elif ch == "\\":
            out.append("\\\\")
        elif ch == "\n":
            out.append("\\n")
        elif ch == "\r":
            out.append("\\r")
        elif ch == "\t":
            out.append("\\t")
        else:
            out.append(ch)
    out.append('"')
    return "".join(out)
